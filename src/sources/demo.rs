//! Temporary demo device.
//!
//! A machine may have no mouse or keyboard that any source can see — the
//! development machine does not, see `docs/devices.md` — which makes the tray
//! and the notifications impossible to check. Setting `BATTERY_TRAY_DEMO_LEVEL`
//! adds one synthetic device so the icons, menu, updates and the low-battery
//! alert can be inspected on a real desktop.
//!
//! The source exists only until the vendor backends of stages 6–8 provide real
//! devices.

use std::env;
use std::sync::atomic::{AtomicU8, Ordering};

use tracing::warn;

use super::{BatterySource, Device, DeviceKind, Protocol, SourceError};

/// Environment variable that enables the demo device and sets its level.
pub const LEVEL_ENV: &str = "BATTERY_TRAY_DEMO_LEVEL";

/// Environment variable that overrides the demo device name.
pub const NAME_ENV: &str = "BATTERY_TRAY_DEMO_NAME";

/// Environment variable that makes the demo level drop by N percent per poll.
pub const STEP_ENV: &str = "BATTERY_TRAY_DEMO_STEP";

/// Value of [`LEVEL_ENV`] that makes the demo device report no charge.
pub const NO_DATA: &str = "none";

const DEMO_DEVICE_ID: &str = "demo-device";
const DEFAULT_NAME: &str = "Demo Mouse";

/// Sentinel for "no level"; a real level never exceeds 100.
const UNKNOWN: u8 = u8::MAX;

/// One synthetic device with a charge level that may fall over time.
pub struct DemoSource {
    name: String,
    level: AtomicU8,
    /// Percent dropped on every poll; `0` keeps the level constant.
    step: u8,
}

impl DemoSource {
    /// Creates the demo source from the environment.
    ///
    /// Returns `None` when the level variable is unset or holds something that
    /// is neither a percentage nor [`NO_DATA`]; a wrong value is not fatal.
    pub fn from_env() -> Option<Self> {
        let value = env::var(LEVEL_ENV).ok()?;
        let name = env::var(NAME_ENV).unwrap_or_else(|_| DEFAULT_NAME.to_owned());
        let step = env::var(STEP_ENV).ok().and_then(|step| parse_step(&step));

        Self::parse(&value, name, step.unwrap_or_default())
    }

    /// Parses a level value: `0..=100`, or [`NO_DATA`] for a device without a
    /// readable charge.
    fn parse(value: &str, name: String, step: u8) -> Option<Self> {
        let trimmed = value.trim();

        let level = if trimmed.eq_ignore_ascii_case(NO_DATA) {
            UNKNOWN
        } else {
            match trimmed.parse::<u8>() {
                Ok(level) if level <= 100 => level,
                _ => {
                    warn!(
                        "{LEVEL_ENV}={value} is not a percentage or `{NO_DATA}`, demo device disabled"
                    );
                    return None;
                }
            }
        };

        Some(Self {
            name,
            level: AtomicU8::new(level),
            step,
        })
    }

    /// Current level without changing it; tests use it to avoid consuming a step.
    #[cfg(test)]
    fn current(&self) -> Option<u8> {
        match self.level.load(Ordering::Relaxed) {
            UNKNOWN => None,
            level => Some(level),
        }
    }
}

/// Reads the per-poll drop; a wrong value means "keep the level constant".
fn parse_step(value: &str) -> Option<u8> {
    match value.trim().parse::<u8>() {
        Ok(step) if step <= 100 => Some(step),
        _ => {
            warn!("{STEP_ENV}={value} is not a percentage, the demo level stays constant");
            None
        }
    }
}

impl BatterySource for DemoSource {
    fn name(&self) -> &str {
        "demo"
    }

    fn list(&self) -> Result<Vec<Device>, SourceError> {
        Ok(vec![Device::new(
            DEMO_DEVICE_ID,
            self.name.clone(),
            DeviceKind::Mouse,
            Protocol::Other,
        )])
    }

    fn level(&self, _dev: &Device) -> Result<Option<u8>, SourceError> {
        let current = self.level.load(Ordering::Relaxed);

        // The drop happens after the reading is taken, so the first poll reports
        // the configured level and every poll after it reports one step less.
        if current != UNKNOWN && self.step > 0 {
            self.level
                .store(current.saturating_sub(self.step), Ordering::Relaxed);
        }

        Ok(match current {
            UNKNOWN => None,
            level => Some(level),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo(value: &str, step: u8) -> DemoSource {
        DemoSource::parse(value, "Demo".to_owned(), step).expect("valid level")
    }

    #[test]
    fn level_is_parsed_as_a_percentage() {
        let source = demo("42", 0);

        assert_eq!(source.current(), Some(42));
        assert_eq!(source.name, "Demo");
    }

    #[test]
    fn boundaries_are_accepted() {
        assert_eq!(demo("0", 0).current(), Some(0));
        assert_eq!(demo("100", 0).current(), Some(100));
    }

    #[test]
    fn none_means_no_data() {
        assert_eq!(demo(NO_DATA, 0).current(), None);
    }

    #[test]
    fn surrounding_spaces_and_case_are_ignored() {
        assert!(DemoSource::parse(" 7 ", "Demo".to_owned(), 0).is_some());
        assert!(DemoSource::parse("NONE", "Demo".to_owned(), 0).is_some());
    }

    #[test]
    fn invalid_values_disable_the_demo_device() {
        assert!(DemoSource::parse("101", "Demo".to_owned(), 0).is_none());
        assert!(DemoSource::parse("-1", "Demo".to_owned(), 0).is_none());
        assert!(DemoSource::parse("42%", "Demo".to_owned(), 0).is_none());
        assert!(DemoSource::parse("", "Demo".to_owned(), 0).is_none());
    }

    #[test]
    fn source_reports_one_device_with_the_given_name() {
        let source = demo("55", 0);
        let devices = source.list().expect("demo never fails");

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Demo");
        assert_eq!(devices[0].kind, DeviceKind::Mouse);
        assert_eq!(
            source.level(&devices[0]).expect("demo never fails"),
            Some(55)
        );
        assert_eq!(source.name(), "demo");
    }

    #[test]
    fn level_drops_by_the_step_on_every_poll() {
        let source = demo("25", 5);
        let device = source.list().expect("demo never fails").remove(0);

        let levels: Vec<Option<u8>> = (0..4)
            .map(|_| source.level(&device).expect("demo never fails"))
            .collect();

        assert_eq!(levels, [Some(25), Some(20), Some(15), Some(10)]);
    }

    #[test]
    fn level_stops_at_zero() {
        let source = demo("4", 3);
        let device = source.list().expect("demo never fails").remove(0);

        let levels: Vec<Option<u8>> = (0..3)
            .map(|_| source.level(&device).expect("demo never fails"))
            .collect();

        assert_eq!(levels, [Some(4), Some(1), Some(0)]);
    }

    #[test]
    fn no_data_stays_no_data() {
        let source = demo(NO_DATA, 5);
        let device = source.list().expect("demo never fails").remove(0);

        assert_eq!(source.level(&device).expect("demo never fails"), None);
        assert_eq!(source.level(&device).expect("demo never fails"), None);
    }

    #[test]
    fn step_is_parsed_and_ignored_when_wrong() {
        assert_eq!(parse_step("5"), Some(5));
        assert_eq!(parse_step(" 0 "), Some(0));
        assert_eq!(parse_step("101"), None);
        assert_eq!(parse_step("fast"), None);
    }
}
