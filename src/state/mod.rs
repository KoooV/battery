//! Device state kept between polls.

use std::time::Duration;

/// How often every source is polled.
pub const POLL_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// How soon a poll is retried when a device reported no level.
///
/// A wireless mouse that sleeps behind its receiver answers nothing, so its
/// last level goes stale; retrying soon keeps the tray current within half a
/// minute of the device waking instead of within ten minutes.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// Charge level, in percent, at which a low-battery notification is sent.
pub const LOW_THRESHOLD: u8 = 10;

/// What the application remembers about one device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceState {
    /// Last successfully read charge level; `None` while the device is silent.
    pub level: Option<u8>,
    /// Set once a low-battery notification has been sent for the current
    /// discharge cycle, so the alert is not repeated on every poll.
    pub last_notified_below_threshold: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_interval_is_ten_minutes() {
        assert_eq!(POLL_INTERVAL, Duration::from_secs(600));
    }

    #[test]
    fn low_threshold_is_ten_percent() {
        assert_eq!(LOW_THRESHOLD, 10);
    }

    #[test]
    fn fresh_state_has_no_level_and_no_notification() {
        let state = DeviceState::default();

        assert_eq!(state.level, None);
        assert!(!state.last_notified_below_threshold);
    }
}
