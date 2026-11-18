//! Polling: asks every source for devices and keeps the tray in sync.
//!
//! The loop is split in two halves: [`poll_once`] reads the sources and [`sync`]
//! translates the result into tray changes. That keeps the bus away from the
//! tray callbacks and lets both halves run against a fake source and a spy sink
//! in tests, without hardware or D-Bus.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::notify::Notifier;
use crate::sources::{BatterySource, Device};

/// How often the loop looks at the stop flag while waiting for the next poll.
/// Short enough for a responsive "Quit", long enough not to busy-wait.
const STOP_CHECK: Duration = Duration::from_millis(200);

/// One device with the level read for it in this poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// Device that was reported.
    pub device: Device,
    /// Level read for it; `None` when the source could not read one.
    pub level: Option<u8>,
}

impl Reading {
    /// Builds a reading for a device.
    pub fn new(device: Device, level: Option<u8>) -> Self {
        Self { device, level }
    }
}

/// Where poll results go: the real tray in the application, a spy in tests.
pub trait TraySink {
    /// Makes the sink show this reading, adding an icon or updating the existing
    /// one.
    fn show(&mut self, reading: &Reading);

    /// Removes the icon of a device that is no longer reported.
    fn remove(&mut self, device_id: &str);

    /// Ids of the devices currently shown.
    fn visible(&self) -> Vec<String>;
}

/// Polls every source once.
///
/// A source that fails to list its devices is logged and skipped; a single
/// device that cannot be read is reported as "no data" instead of losing its
/// icon. Neither case aborts the poll, so one broken backend cannot take the
/// whole tray down.
pub fn poll_once(sources: &[Box<dyn BatterySource>]) -> Vec<Reading> {
    let mut readings = Vec::new();

    for source in sources {
        let devices = match source.list() {
            Ok(devices) => devices,
            Err(err) => {
                warn!("{err}");
                continue;
            }
        };

        for device in devices {
            let level = match source.level(&device) {
                Ok(level) => level,
                Err(err) => {
                    warn!("{err}");
                    None
                }
            };
            readings.push(Reading::new(device, level));
        }
    }

    readings
}

/// Brings the tray in line with a poll result.
///
/// Devices in `readings` are shown (added or updated), devices that were visible
/// but are missing from the result are removed. Returns the ids that disappeared,
/// which the notification logic will need in stage 5.
pub fn sync(sink: &mut impl TraySink, readings: &[Reading]) -> Vec<String> {
    for reading in readings {
        sink.show(reading);
    }

    let reported: Vec<&str> = readings
        .iter()
        .map(|reading| reading.device.id.as_str())
        .collect();
    let gone: Vec<String> = sink
        .visible()
        .into_iter()
        .filter(|id| !reported.contains(&id.as_str()))
        .collect();

    for id in &gone {
        sink.remove(id);
    }

    gone
}

/// Polls immediately, then after `interval` — while some device reports no level
/// the gap grows through `retry` instead — until `stop` is set.
///
/// Runs on the caller's thread, which must not be inside an async runtime: the
/// sources drive D-Bus with `zbus::block_on`.
pub fn run<S: TraySink>(
    sources: &[Box<dyn BatterySource>],
    sink: &mut S,
    notifier: &mut Notifier,
    interval: Duration,
    retry: &[Duration],
    stop: &AtomicBool,
) {
    let mut schedule = Schedule::new(interval, retry);

    loop {
        let readings = poll_once(sources);
        log_readings(&readings);
        sync(sink, &readings);
        notify(notifier, &readings);

        if stop.load(Ordering::Relaxed) {
            return;
        }

        // A stop request during the wait ends the loop without another poll: the
        // extra poll would only delay the exit behind another D-Bus round trip.
        if wait(schedule.next_delay(&readings), stop) {
            return;
        }
    }
}

/// Gap before the next poll.
///
/// A device without a level is usually asleep behind its receiver, so the level
/// it left in the tray is stale rather than gone and polling again soon shows
/// the fresh value once the device wakes. A device that stays silent is not
/// worth a query every half minute, so the gap grows along `retry` up to its
/// last entry; one successful read puts the loop back on `interval`.
struct Schedule {
    interval: Duration,
    retry: Vec<Duration>,
    silent_polls: usize,
}

impl Schedule {
    fn new(interval: Duration, retry: &[Duration]) -> Self {
        Self {
            interval,
            retry: retry.to_vec(),
            silent_polls: 0,
        }
    }

    /// Delay before the next poll, given what the last poll returned.
    fn next_delay(&mut self, readings: &[Reading]) -> Duration {
        if !readings.iter().any(|reading| reading.level.is_none()) {
            self.silent_polls = 0;
            return self.interval;
        }

        let step = self.silent_polls.min(self.retry.len().saturating_sub(1));
        self.silent_polls = self.silent_polls.saturating_add(1);

        self.retry.get(step).copied().unwrap_or(self.interval)
    }
}

/// Feeds the notifier and shows the alerts it returns.
fn notify(notifier: &mut Notifier, readings: &[Reading]) {
    for reading in readings {
        if let Some(alert) = notifier.observe(&reading.device, reading.level) {
            notifier.send(&alert);
        }
    }
}

/// Info per device, so the log shows what the tray should display.
fn log_readings(readings: &[Reading]) {
    if readings.is_empty() {
        debug!("no device with a battery reported");
        return;
    }

    for reading in readings {
        match reading.level {
            Some(level) => info!("{}: {level}%", reading.device.name),
            // Not `info`: while a device sleeps this repeats on every retry.
            None => debug!("{}: no data", reading.device.name),
        }
    }
}

/// Sleeps up to `interval`, waking early when a stop is requested.
///
/// Returns `true` when the caller should stop.
fn wait(interval: Duration, stop: &AtomicBool) -> bool {
    let mut slept = Duration::ZERO;

    while slept < interval {
        if stop.load(Ordering::Relaxed) {
            return true;
        }

        let step = STOP_CHECK.min(interval - slept);
        thread::sleep(step);
        slept += step;
    }

    stop.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::sources::{DeviceKind, Protocol, SourceError};

    /// Stand-ins for the real constants, so a change to them cannot break a test.
    const REGULAR: Duration = Duration::from_secs(600);
    const BACKOFF: [Duration; 3] = [
        Duration::from_secs(30),
        Duration::from_secs(60),
        Duration::from_secs(300),
    ];

    fn schedule() -> Schedule {
        Schedule::new(REGULAR, &BACKOFF)
    }

    fn silent() -> [Reading; 1] {
        [Reading::new(device("mouse", "Mouse"), None)]
    }

    /// Source that serves canned data and can be told to fail.
    struct FakeSource {
        devices: Vec<Device>,
        level: Option<u8>,
        fail_list: bool,
        fail_level: bool,
    }

    impl FakeSource {
        fn new(devices: Vec<Device>, level: Option<u8>) -> Self {
            Self {
                devices,
                level,
                fail_list: false,
                fail_level: false,
            }
        }

        fn failing_list() -> Self {
            Self {
                fail_list: true,
                ..Self::new(Vec::new(), None)
            }
        }

        fn failing_level(devices: Vec<Device>) -> Self {
            Self {
                fail_level: true,
                ..Self::new(devices, None)
            }
        }
    }

    impl BatterySource for FakeSource {
        fn name(&self) -> &str {
            "fake"
        }

        fn list(&self) -> Result<Vec<Device>, SourceError> {
            if self.fail_list {
                return Err(SourceError::Unavailable {
                    backend: "fake",
                    reason: "no daemon".to_owned(),
                });
            }
            Ok(self.devices.clone())
        }

        fn level(&self, device: &Device) -> Result<Option<u8>, SourceError> {
            if self.fail_level {
                return Err(SourceError::Read {
                    device: device.id.clone(),
                    reason: "gone".to_owned(),
                });
            }
            Ok(self.level)
        }
    }

    /// Sink that records what the tray would have done.
    #[derive(Default)]
    struct SpySink {
        shown: Vec<(String, Option<u8>)>,
        removed: Vec<String>,
        visible: Vec<String>,
    }

    impl TraySink for SpySink {
        fn show(&mut self, reading: &Reading) {
            self.shown.push((reading.device.id.clone(), reading.level));
            if !self.visible.contains(&reading.device.id) {
                self.visible.push(reading.device.id.clone());
            }
        }

        fn remove(&mut self, device_id: &str) {
            self.removed.push(device_id.to_owned());
            self.visible.retain(|id| id != device_id);
        }

        fn visible(&self) -> Vec<String> {
            self.visible.clone()
        }
    }

    fn device(id: &str, name: &str) -> Device {
        Device::new(id, name, DeviceKind::Mouse, Protocol::Upower)
    }

    #[test]
    fn poll_reads_every_device_of_every_source() {
        let sources: Vec<Box<dyn BatterySource>> = vec![
            Box::new(FakeSource::new(vec![device("mouse", "Mouse")], Some(40))),
            Box::new(FakeSource::new(vec![device("keys", "Keyboard")], Some(80))),
        ];

        let readings = poll_once(&sources);

        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].device.id, "mouse");
        assert_eq!(readings[0].level, Some(40));
        assert_eq!(readings[1].device.id, "keys");
        assert_eq!(readings[1].level, Some(80));
    }

    #[test]
    fn failing_source_does_not_stop_the_poll() {
        let sources: Vec<Box<dyn BatterySource>> = vec![
            Box::new(FakeSource::failing_list()),
            Box::new(FakeSource::new(vec![device("mouse", "Mouse")], Some(40))),
        ];

        let readings = poll_once(&sources);

        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].device.id, "mouse");
    }

    #[test]
    fn unreadable_device_is_reported_as_no_data() {
        let sources: Vec<Box<dyn BatterySource>> =
            vec![Box::new(FakeSource::failing_level(vec![device(
                "mouse", "Mouse",
            )]))];

        let readings = poll_once(&sources);

        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].level, None);
    }

    #[test]
    fn new_devices_get_an_icon() {
        let mut sink = SpySink::default();
        let readings = [
            Reading::new(device("mouse", "Mouse"), Some(40)),
            Reading::new(device("keys", "Keyboard"), Some(80)),
        ];

        let gone = sync(&mut sink, &readings);

        assert_eq!(
            sink.shown,
            [
                ("mouse".to_owned(), Some(40)),
                ("keys".to_owned(), Some(80))
            ]
        );
        assert!(gone.is_empty());
        assert!(sink.removed.is_empty());
        assert_eq!(sink.visible(), ["mouse", "keys"]);
    }

    #[test]
    fn existing_device_is_updated_in_place() {
        let mut sink = SpySink::default();
        sync(
            &mut sink,
            &[Reading::new(device("mouse", "Mouse"), Some(80))],
        );

        sync(
            &mut sink,
            &[Reading::new(device("mouse", "Mouse"), Some(75))],
        );

        assert_eq!(
            sink.shown,
            [
                ("mouse".to_owned(), Some(80)),
                ("mouse".to_owned(), Some(75))
            ]
        );
        assert!(sink.removed.is_empty());
        assert_eq!(sink.visible(), ["mouse"]);
    }

    #[test]
    fn device_that_disappears_loses_its_icon() {
        let mut sink = SpySink::default();
        sync(
            &mut sink,
            &[
                Reading::new(device("mouse", "Mouse"), Some(40)),
                Reading::new(device("keys", "Keyboard"), Some(80)),
            ],
        );

        let gone = sync(
            &mut sink,
            &[Reading::new(device("keys", "Keyboard"), Some(79))],
        );

        assert_eq!(gone, ["mouse"]);
        assert_eq!(sink.removed, ["mouse"]);
        assert_eq!(sink.visible(), ["keys"]);
    }

    #[test]
    fn device_that_comes_back_gets_a_new_icon() {
        let mut sink = SpySink::default();
        sync(
            &mut sink,
            &[Reading::new(device("mouse", "Mouse"), Some(40))],
        );
        sync(&mut sink, &[]);

        let gone = sync(
            &mut sink,
            &[Reading::new(device("mouse", "Mouse"), Some(39))],
        );

        assert!(gone.is_empty());
        assert_eq!(sink.removed, ["mouse"]);
        assert_eq!(sink.visible(), ["mouse"]);
    }

    #[test]
    fn silent_device_keeps_its_icon_without_data() {
        let mut sink = SpySink::default();
        sync(
            &mut sink,
            &[Reading::new(device("mouse", "Mouse"), Some(40))],
        );

        sync(&mut sink, &[Reading::new(device("mouse", "Mouse"), None)]);

        assert!(sink.removed.is_empty());
        assert_eq!(sink.shown.last(), Some(&("mouse".to_owned(), None)));
    }

    #[test]
    fn wait_returns_at_once_when_stop_is_already_set() {
        let stop = AtomicBool::new(true);

        assert!(wait(Duration::from_secs(60), &stop));
    }

    #[test]
    fn wait_gives_up_early_when_stopped_meanwhile() {
        let stop = Arc::new(AtomicBool::new(false));
        let wake = Arc::clone(&stop);
        let stopper = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            wake.store(true, Ordering::Relaxed);
        });

        let started = std::time::Instant::now();
        let stopped = wait(Duration::from_secs(60), &stop);
        stopper.join().expect("stopper thread");

        assert!(stopped);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn wait_sleeps_through_a_short_interval() {
        let stop = AtomicBool::new(false);

        assert!(!wait(Duration::from_millis(10), &stop));
    }

    #[test]
    fn silent_device_is_retried_after_the_shortest_gap() {
        assert_eq!(
            schedule().next_delay(&silent()),
            BACKOFF[0],
            "a sleeping device must not wait for the regular interval"
        );
    }

    #[test]
    fn one_silent_device_is_enough_to_retry() {
        let readings = [
            Reading::new(device("mouse", "Mouse"), Some(80)),
            Reading::new(device("keys", "Keyboard"), None),
        ];

        assert_eq!(schedule().next_delay(&readings), BACKOFF[0]);
    }

    #[test]
    fn repeated_silence_stretches_the_gap_and_then_holds() {
        let mut schedule = schedule();
        let delays: Vec<Duration> = (0..5).map(|_| schedule.next_delay(&silent())).collect();

        assert_eq!(
            delays,
            [BACKOFF[0], BACKOFF[1], BACKOFF[2], BACKOFF[2], BACKOFF[2]]
        );
    }

    #[test]
    fn successful_read_returns_to_the_regular_interval() {
        let mut schedule = schedule();
        let awake = [Reading::new(device("mouse", "Mouse"), Some(77))];

        assert_eq!(schedule.next_delay(&silent()), BACKOFF[0]);
        assert_eq!(schedule.next_delay(&silent()), BACKOFF[1]);
        assert_eq!(schedule.next_delay(&awake), REGULAR);
        assert_eq!(
            schedule.next_delay(&silent()),
            BACKOFF[0],
            "a new silence starts over from the shortest gap"
        );
    }

    #[test]
    fn poll_with_every_level_waits_the_regular_interval() {
        let readings = [
            Reading::new(device("mouse", "Mouse"), Some(80)),
            Reading::new(device("keys", "Keyboard"), Some(40)),
        ];

        assert_eq!(schedule().next_delay(&readings), REGULAR);
    }

    #[test]
    fn empty_poll_waits_the_regular_interval() {
        assert_eq!(schedule().next_delay(&[]), REGULAR);
    }

    #[test]
    fn missing_backoff_never_retries_sooner_than_the_interval() {
        let mut schedule = Schedule::new(REGULAR, &[]);

        assert_eq!(schedule.next_delay(&silent()), REGULAR);
    }

    #[test]
    fn empty_poll_removes_everything() {
        let mut sink = SpySink::default();
        sync(
            &mut sink,
            &[
                Reading::new(device("a", "A"), Some(1)),
                Reading::new(device("b", "B"), Some(2)),
            ],
        );

        let gone = sync(&mut sink, &[]);

        assert_eq!(gone.len(), 2);
        assert!(sink.visible().is_empty());
    }
}
