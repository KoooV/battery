//! Desktop notifications for a low battery.
//!
//! One discharge cycle produces one alert. The alert fires when a level drops
//! from above the threshold to the threshold or below, and does not fire again
//! until the device has been charged above the threshold.
//!
//! A device that is already at or below the threshold when the application
//! starts is not announced. Otherwise every login with an almost empty battery
//! would produce a notification, and the tray already shows that level — the
//! alert is there to warn about a drop the user did not see. This is the
//! decision PLAN stage 5 asked for; turning it around is one branch in
//! [`Notifier::observe`].

use std::collections::HashMap;

use notify_rust::Notification;
use tracing::{info, warn};

use crate::sources::Device;
use crate::state::DeviceState;

/// Text of a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    /// Notification title.
    pub summary: String,
    /// Notification body.
    pub body: String,
}

impl Alert {
    /// Alert for a device that just reached the low threshold.
    pub fn low_battery(device: &Device, level: u8) -> Self {
        Self {
            summary: format!("{}: {level}%", device.name),
            body: "Battery is low, charge the device.".to_owned(),
        }
    }
}

/// Delivers an alert to the desktop.
pub trait AlertSender {
    /// Shows the alert; the error describes why it could not be shown.
    fn send(&self, alert: &Alert) -> Result<(), String>;
}

/// Sends alerts through the desktop notification daemon.
pub struct DesktopAlerts;

impl AlertSender for DesktopAlerts {
    fn send(&self, alert: &Alert) -> Result<(), String> {
        Notification::new()
            .appname("battery-tray")
            .summary(&alert.summary)
            .body(&alert.body)
            .show()
            .map(|_| ())
            .map_err(|err| format!("cannot show a notification: {err}"))
    }
}

/// Remembers the last level of every device and decides when to alert.
pub struct Notifier {
    threshold: u8,
    sender: Box<dyn AlertSender>,
    states: HashMap<String, DeviceState>,
}

impl Notifier {
    /// Notifier that talks to the desktop notification daemon.
    pub fn new(threshold: u8) -> Self {
        Self::with_sender(threshold, Box::new(DesktopAlerts))
    }

    /// Notifier that hands alerts to `sender`; tests use this with a spy.
    pub fn with_sender(threshold: u8, sender: Box<dyn AlertSender>) -> Self {
        Self {
            threshold,
            sender,
            states: HashMap::new(),
        }
    }

    /// Threshold this notifier alerts at.
    pub fn threshold(&self) -> u8 {
        self.threshold
    }

    /// Feeds the level read for a device in one poll.
    ///
    /// Returns the alert to show when the level just crossed the threshold.
    /// `None` means "stay quiet": the level did not cross, it is already
    /// announced, or this is the first reading of the device.
    pub fn observe(&mut self, device: &Device, level: Option<u8>) -> Option<Alert> {
        let state = self.states.entry(device.id.clone()).or_default();
        let previous = state.level;
        state.level = level;

        let level = level?;

        if level > self.threshold {
            // Charged above the threshold: the next drop is a new discharge
            // cycle and may be announced again.
            state.last_notified_below_threshold = false;
            return None;
        }

        if state.last_notified_below_threshold {
            return None;
        }

        // `None` means the device was not seen before, or was silent for a
        // while; both are treated as a first reading, not as a drop.
        let crossed = previous.is_some_and(|previous| previous > self.threshold);
        if !crossed {
            return None;
        }

        state.last_notified_below_threshold = true;
        Some(Alert::low_battery(device, level))
    }

    /// Shows the alert and logs the outcome; a missing daemon is not fatal.
    pub fn send(&self, alert: &Alert) {
        match self.sender.send(alert) {
            Ok(()) => info!("notified about {}", alert.summary),
            Err(err) => warn!("{err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::sources::{DeviceKind, Protocol};

    fn device() -> Device {
        Device::new("mouse", "Test Mouse", DeviceKind::Mouse, Protocol::Upower)
    }

    /// Sender that keeps the alerts instead of showing them.
    #[derive(Clone, Default)]
    struct SpySender {
        alerts: Arc<Mutex<Vec<Alert>>>,
        fail: bool,
    }

    impl AlertSender for SpySender {
        fn send(&self, alert: &Alert) -> Result<(), String> {
            if self.fail {
                return Err("no notification daemon".to_owned());
            }
            self.alerts.lock().expect("test lock").push(alert.clone());
            Ok(())
        }
    }

    fn notifier(threshold: u8) -> (Notifier, SpySender) {
        let spy = SpySender::default();
        (Notifier::with_sender(threshold, Box::new(spy.clone())), spy)
    }

    /// Feeds a list of levels and says which of them alerted.
    fn observe_all(notifier: &mut Notifier, levels: &[Option<u8>]) -> Vec<Option<u8>> {
        levels
            .iter()
            .map(|&level| {
                notifier
                    .observe(&device(), level)
                    .map(|_| level.expect("an alert always has a level"))
            })
            .collect()
    }

    #[test]
    fn drop_through_the_threshold_alerts_once() {
        let (mut notifier, _) = notifier(10);

        let alerts = observe_all(&mut notifier, &[Some(50), Some(11), Some(10)]);

        assert_eq!(alerts, [None, None, Some(10)]);
    }

    #[test]
    fn staying_below_the_threshold_stays_quiet() {
        let (mut notifier, _) = notifier(10);

        let alerts = observe_all(
            &mut notifier,
            &[Some(11), Some(10), Some(9), Some(8), Some(8)],
        );

        assert_eq!(alerts, [None, Some(10), None, None, None]);
    }

    #[test]
    fn charging_above_the_threshold_rearms_the_alert() {
        let (mut notifier, _) = notifier(10);

        let alerts = observe_all(
            &mut notifier,
            &[Some(20), Some(9), Some(9), Some(60), Some(9)],
        );

        assert_eq!(alerts, [None, Some(9), None, None, Some(9)]);
    }

    #[test]
    fn level_above_the_threshold_never_alerts() {
        let (mut notifier, _) = notifier(10);

        let alerts = observe_all(&mut notifier, &[Some(100), Some(90), Some(11), Some(11)]);

        assert_eq!(alerts, [None, None, None, None]);
    }

    #[test]
    fn first_reading_below_the_threshold_does_not_alert() {
        let (mut notifier, _) = notifier(10);

        let alerts = observe_all(&mut notifier, &[Some(4), Some(4), Some(3)]);

        assert_eq!(alerts, [None, None, None]);
    }

    #[test]
    fn exactly_at_the_threshold_counts_as_low() {
        let (mut notifier, _) = notifier(10);

        assert!(notifier.observe(&device(), Some(12)).is_none());
        assert!(notifier.observe(&device(), Some(10)).is_some());
    }

    #[test]
    fn missing_level_does_not_alert_and_does_not_rearm() {
        let (mut notifier, _) = notifier(10);

        // Low, then silent, then low again: no second alert, the silence is not
        // a crossing.
        let alerts = observe_all(&mut notifier, &[Some(50), Some(9), None, Some(9)]);

        assert_eq!(alerts, [None, Some(9), None, None]);
    }

    #[test]
    fn device_seen_after_a_gap_is_treated_as_a_first_reading() {
        let (mut notifier, _) = notifier(10);

        // The device disappears (silent) while above the threshold, then shows up
        // low: without a previous level there is no drop to announce.
        let alerts = observe_all(&mut notifier, &[Some(80), None, Some(7)]);

        assert_eq!(alerts, [None, None, None]);
    }

    #[test]
    fn devices_keep_separate_state() {
        let (mut notifier, _) = notifier(10);
        let mouse = Device::new("mouse", "Mouse", DeviceKind::Mouse, Protocol::Upower);
        let keys = Device::new("keys", "Keyboard", DeviceKind::Keyboard, Protocol::Hidpp);

        assert!(notifier.observe(&mouse, Some(30)).is_none());
        assert!(notifier.observe(&keys, Some(30)).is_none());
        assert!(notifier.observe(&mouse, Some(10)).is_some());
        assert!(
            notifier.observe(&keys, Some(10)).is_some(),
            "the other device has its own discharge cycle"
        );
    }

    #[test]
    fn alert_text_names_the_device_and_the_level() {
        let (mut notifier, spy) = notifier(10);
        notifier.observe(&device(), Some(40));
        let alert = notifier.observe(&device(), Some(10)).expect("alert");

        assert_eq!(alert.summary, "Test Mouse: 10%");
        assert!(alert.body.contains("charge"), "body: {}", alert.body);

        notifier.send(&alert);
        assert_eq!(spy.alerts.lock().expect("test lock").len(), 1);
    }

    #[test]
    fn failed_delivery_does_not_panic() {
        let spy = SpySender {
            fail: true,
            ..SpySender::default()
        };
        let notifier = Notifier::with_sender(10, Box::new(spy));

        // The daemon is missing: the alert is dropped with a warning.
        notifier.send(&Alert::low_battery(&device(), 5));
    }

    #[test]
    fn threshold_is_exposed() {
        assert_eq!(notifier(7).0.threshold(), 7);
    }
}
