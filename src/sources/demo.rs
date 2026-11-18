//! Temporary demo device.
//!
//! A machine may have no mouse or keyboard that any source can see — the
//! development machine does not, see `docs/devices.md` — which makes the tray
//! impossible to check. Setting `BATTERY_TRAY_DEMO_LEVEL` adds one synthetic
//! device so the icons, menu and updates can be inspected on a real desktop.
//!
//! The source exists only until the vendor backends of stages 6–8 provide real
//! devices.

use std::env;

use tracing::warn;

use super::{BatterySource, Device, DeviceKind, Protocol, SourceError};

/// Environment variable that enables the demo device and sets its level.
pub const LEVEL_ENV: &str = "BATTERY_TRAY_DEMO_LEVEL";

/// Environment variable that overrides the demo device name.
pub const NAME_ENV: &str = "BATTERY_TRAY_DEMO_NAME";

/// Value of [`LEVEL_ENV`] that makes the demo device report no charge.
pub const NO_DATA: &str = "none";

const DEMO_DEVICE_ID: &str = "demo-device";
const DEFAULT_NAME: &str = "Demo Mouse";

/// One synthetic device with a fixed charge level.
pub struct DemoSource {
    name: String,
    level: Option<u8>,
}

impl DemoSource {
    /// Creates the demo source from the environment.
    ///
    /// Returns `None` when the variable is unset or holds something that is
    /// neither a percentage nor [`NO_DATA`]; a wrong value is not fatal.
    pub fn from_env() -> Option<Self> {
        let value = env::var(LEVEL_ENV).ok()?;
        let name = env::var(NAME_ENV).unwrap_or_else(|_| DEFAULT_NAME.to_owned());

        Self::parse(&value, name)
    }

    /// Parses a level value: `0..=100`, or [`NO_DATA`] for a device without a
    /// readable charge.
    fn parse(value: &str, name: String) -> Option<Self> {
        let trimmed = value.trim();

        if trimmed.eq_ignore_ascii_case(NO_DATA) {
            return Some(Self { name, level: None });
        }

        match trimmed.parse::<u8>() {
            Ok(level) if level <= 100 => Some(Self {
                name,
                level: Some(level),
            }),
            _ => {
                warn!(
                    "{LEVEL_ENV}={value} is not a percentage or `{NO_DATA}`, demo device disabled"
                );
                None
            }
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
        Ok(self.level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_is_parsed_as_a_percentage() {
        let source = DemoSource::parse("42", "Demo".to_owned()).expect("42 is valid");

        assert_eq!(source.level, Some(42));
        assert_eq!(source.name, "Demo");
    }

    #[test]
    fn boundaries_are_accepted() {
        assert_eq!(
            DemoSource::parse("0", "Demo".to_owned()).map(|s| s.level),
            Some(Some(0))
        );
        assert_eq!(
            DemoSource::parse("100", "Demo".to_owned()).map(|s| s.level),
            Some(Some(100))
        );
    }

    #[test]
    fn none_means_no_data() {
        let source = DemoSource::parse("none", "Demo".to_owned()).expect("none is valid");

        assert_eq!(source.level, None);
    }

    #[test]
    fn surrounding_spaces_and_case_are_ignored() {
        assert!(DemoSource::parse(" 7 ", "Demo".to_owned()).is_some());
        assert!(DemoSource::parse("NONE", "Demo".to_owned()).is_some());
    }

    #[test]
    fn invalid_values_disable_the_demo_device() {
        assert!(DemoSource::parse("101", "Demo".to_owned()).is_none());
        assert!(DemoSource::parse("-1", "Demo".to_owned()).is_none());
        assert!(DemoSource::parse("42%", "Demo".to_owned()).is_none());
        assert!(DemoSource::parse("", "Demo".to_owned()).is_none());
    }

    #[test]
    fn source_reports_one_device_with_the_given_name() {
        let source = DemoSource::parse("55", "Demo Keyboard".to_owned()).expect("valid");
        let devices = source.list().expect("demo never fails");

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Demo Keyboard");
        assert_eq!(devices[0].kind, DeviceKind::Mouse);
        assert_eq!(
            source.level(&devices[0]).expect("demo never fails"),
            Some(55)
        );
        assert_eq!(source.name(), "demo");
    }
}
