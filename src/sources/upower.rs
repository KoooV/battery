//! UPower battery source.
//!
//! UPower publishes every power supply it knows as an object on the system bus
//! under `/org/freedesktop/UPower/devices/`. The source enumerates those objects
//! and keeps the peripherals, dropping the laptop battery, line power and the
//! other kinds a laptop reports about itself.
//!
//! zbus 6 has no blocking API, so the D-Bus calls are driven with
//! [`zbus::block_on`]. That helper must not be called from inside another async
//! runtime's task; the polling loop runs on a plain thread, which satisfies it.

use std::collections::HashMap;
use std::fmt::Display;
use std::time::Duration;

use zbus::{OwnedObjectPath, OwnedVariant, block_on};

use super::{BatterySource, Device, DeviceKind, Protocol, SourceError};

/// Name of this backend in logs and errors.
const BACKEND: &str = "upower";
/// Well-known bus name of the UPower daemon.
const DESTINATION: &str = "org.freedesktop.UPower";
/// Object that owns `EnumerateDevices`.
const MANAGER_PATH: &str = "/org/freedesktop/UPower";
/// Interface carrying the device properties.
const DEVICE_INTERFACE: &str = "org.freedesktop.UPower.Device";
/// Standard D-Bus interface that exposes `GetAll`.
const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";

/// Bound for every D-Bus call: a hung or unresponsive daemon must not stall the
/// polling loop.
const METHOD_TIMEOUT: Duration = Duration::from_secs(5);

/// Values of the `Type` property, from UPower's `UpDeviceKind` enum.
mod upower_type {
    pub const MOUSE: u32 = 5;
    pub const KEYBOARD: u32 = 6;
    pub const GAMING_INPUT: u32 = 12;
    pub const PEN: u32 = 13;
    pub const TOUCHPAD: u32 = 14;
    pub const HEADSET: u32 = 17;
    pub const HEADPHONES: u32 = 19;
}

/// Maps a UPower `Type` to the kind of peripheral reported to the app.
///
/// Returns `None` for everything that is not a peripheral with a battery of its
/// own — the system battery, line power, UPS, monitor and the rest. Anything
/// unknown stays out too, so a future UPower type cannot silently show up as a
/// mouse.
fn kind_for_upower_type(kind: u32) -> Option<DeviceKind> {
    match kind {
        upower_type::MOUSE | upower_type::TOUCHPAD => Some(DeviceKind::Mouse),
        upower_type::KEYBOARD => Some(DeviceKind::Keyboard),
        upower_type::GAMING_INPUT
        | upower_type::PEN
        | upower_type::HEADSET
        | upower_type::HEADPHONES => Some(DeviceKind::Other),
        _ => None,
    }
}

/// Fallback name for a device whose `Model` property is empty.
fn fallback_name(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Mouse => "Mouse",
        DeviceKind::Keyboard => "Keyboard",
        DeviceKind::Other => "Peripheral",
    }
}

/// Converts UPower's fractional percentage into whole percent.
///
/// `None` when UPower sends something that is not a number. Out-of-range values
/// are clamped, so a broken daemon cannot make the tray print `-3%`.
fn percentage_to_u8(percentage: f64) -> Option<u8> {
    if !percentage.is_finite() {
        return None;
    }

    // The range is tiny, so the cast cannot truncate.
    Some(percentage.clamp(0.0, 100.0).round() as u8)
}

/// The part of an `org.freedesktop.UPower.Device` object this source uses.
#[derive(Debug, Clone, PartialEq)]
struct DeviceSnapshot {
    /// Object path of the device, used as the device id.
    object_path: String,
    /// `Model` property; empty when UPower has no name for the device.
    model: String,
    /// Raw `Type` property.
    kind: u32,
    /// `Percentage` property as a fraction of 100, `None` when it was absent.
    percentage: Option<f64>,
    /// `IsPresent` property.
    is_present: bool,
}

impl DeviceSnapshot {
    /// Describes the device for the rest of the application.
    ///
    /// Returns `None` when the object is not a device worth a tray icon: wrong
    /// kind, or not present right now because it is switched off or out of range
    /// of its receiver.
    fn to_device(&self) -> Option<Device> {
        if !self.is_present {
            return None;
        }

        let kind = kind_for_upower_type(self.kind)?;
        let trimmed = self.model.trim();
        let name = if trimmed.is_empty() {
            fallback_name(kind).to_owned()
        } else {
            trimmed.to_owned()
        };

        Some(Device::new(
            self.object_path.clone(),
            name,
            kind,
            Protocol::Upower,
        ))
    }

    /// Charge level in whole percent, `None` when UPower reports no usable value.
    fn level(&self) -> Option<u8> {
        self.percentage.and_then(percentage_to_u8)
    }
}

/// Reads a property out of a `GetAll` reply.
fn property<T>(properties: &HashMap<String, OwnedVariant>, key: &str) -> Result<T, String>
where
    T: TryFrom<OwnedVariant>,
    <T as TryFrom<OwnedVariant>>::Error: Display,
{
    let value = properties
        .get(key)
        .ok_or_else(|| format!("UPower sent no `{key}` property"))?;

    T::try_from(value.clone()).map_err(|err| format!("`{key}` has an unexpected type: {err}"))
}

/// Reads a property that older UPower versions may not send.
///
/// An absent property is `Ok(None)`; a property that is there but has the wrong
/// type is still an error, so a protocol change cannot hide behind the default.
fn optional_property<T>(
    properties: &HashMap<String, OwnedVariant>,
    key: &str,
) -> Result<Option<T>, String>
where
    T: TryFrom<OwnedVariant>,
    <T as TryFrom<OwnedVariant>>::Error: Display,
{
    if properties.contains_key(key) {
        property(properties, key).map(Some)
    } else {
        Ok(None)
    }
}

/// Builds a snapshot from the `GetAll` reply of one UPower object.
///
/// `Ok(None)` means the object is not a peripheral and is skipped. Property
/// values the source does not strictly need are optional: a device that reports
/// no percentage is shown as "no data" instead of failing the whole poll.
fn snapshot_from_properties(
    object_path: &str,
    properties: &HashMap<String, OwnedVariant>,
) -> Result<Option<DeviceSnapshot>, String> {
    let kind: u32 = property(properties, "Type")?;
    if kind_for_upower_type(kind).is_none() {
        return Ok(None);
    }

    Ok(Some(DeviceSnapshot {
        object_path: object_path.to_owned(),
        model: optional_property(properties, "Model")?.unwrap_or_default(),
        kind,
        percentage: optional_property(properties, "Percentage")?,
        is_present: optional_property(properties, "IsPresent")?.unwrap_or_default(),
    }))
}

/// [`BatterySource`] backed by the UPower daemon.
pub struct UpowerSource {
    connection: zbus::Connection,
}

impl UpowerSource {
    /// Connects to the system bus.
    ///
    /// Only opening the connection happens here; a UPower that is installed but
    /// not running is reported later by [`BatterySource::list`], so a daemon
    /// restart does not need a restart of the application.
    pub fn connect() -> Result<Self, SourceError> {
        let connection = block_on(
            zbus::connection::Builder::system()
                .method_timeout(METHOD_TIMEOUT)
                .build(),
        )
        .map_err(|err| SourceError::Unavailable {
            backend: BACKEND,
            reason: format!("cannot connect to the system bus: {err}"),
        })?;

        Ok(Self { connection })
    }

    /// `org.freedesktop.UPower.EnumerateDevices` on the manager object.
    fn enumerate(&self) -> Result<Vec<OwnedObjectPath>, SourceError> {
        let proxy = block_on(zbus::Proxy::new(
            &self.connection,
            DESTINATION,
            MANAGER_PATH,
            DESTINATION,
        ))
        .map_err(|err| unreadable("UPower manager object", err))?;

        block_on(proxy.call::<_, _, Vec<OwnedObjectPath>>("EnumerateDevices", &())).map_err(|err| {
            SourceError::Unavailable {
                backend: BACKEND,
                reason: format!("EnumerateDevices failed: {err}"),
            }
        })
    }

    /// All properties of one device object.
    fn properties(
        &self,
        path: &OwnedObjectPath,
    ) -> Result<HashMap<String, OwnedVariant>, SourceError> {
        let device = path.to_string();
        // `GetAll` lives on the standard properties interface; calling it through
        // a plain proxy avoids having to build an `InterfaceName` at runtime.
        let proxy = block_on(zbus::Proxy::new(
            &self.connection,
            DESTINATION,
            path.clone(),
            PROPERTIES_INTERFACE,
        ))
        .map_err(|err| unreadable(&device, err))?;

        block_on(proxy.call::<_, _, HashMap<String, OwnedVariant>>("GetAll", &(DEVICE_INTERFACE,)))
            .map_err(|err| unreadable(&device, err))
    }
}

/// Error for a device object that could not be read, e.g. because it was removed
/// between enumeration and reading.
fn unreadable(device: &str, err: impl Display) -> SourceError {
    SourceError::Read {
        device: device.to_owned(),
        reason: err.to_string(),
    }
}

impl BatterySource for UpowerSource {
    fn name(&self) -> &str {
        BACKEND
    }

    fn list(&self) -> Result<Vec<Device>, SourceError> {
        let paths = self.enumerate()?;
        let mut devices = Vec::new();

        for path in paths {
            let device = path.to_string();
            let properties = self.properties(&path)?;

            match snapshot_from_properties(&device, &properties) {
                Ok(Some(snapshot)) => devices.extend(snapshot.to_device()),
                Ok(None) => {}
                Err(reason) => return Err(unreadable(&device, reason)),
            }
        }

        Ok(devices)
    }

    fn level(&self, dev: &Device) -> Result<Option<u8>, SourceError> {
        let path = OwnedObjectPath::try_from(dev.id.clone())
            .map_err(|err| unreadable(&dev.id, format!("not an object path: {err}")))?;
        let properties = self.properties(&path)?;

        Ok(snapshot_from_properties(&dev.id, &properties)
            .map_err(|reason| unreadable(&dev.id, reason))?
            .and_then(|snapshot| snapshot.level()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GetAll` replies saved in `tests/fixtures/`, one block per device.
    fn fixture_blocks() -> Vec<HashMap<String, OwnedVariant>> {
        let text = include_str!("../../tests/fixtures/upower-getall.txt");
        let mut blocks: Vec<HashMap<String, OwnedVariant>> = Vec::new();
        let mut current = HashMap::new();

        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "---" {
                blocks.push(std::mem::take(&mut current));
                continue;
            }

            let (key, raw) = line.split_once('=').expect("fixture lines are `key=value`");
            current.insert(key.to_owned(), fixture_value(key, raw));
        }

        blocks.push(current);
        blocks
    }

    /// Turns a string into the owned value a D-Bus reply would carry.
    fn text(value: &str) -> OwnedVariant {
        OwnedVariant::from(zbus::Str::from(value))
    }

    /// The fixture stores plain text, so each key is typed like UPower types it.
    fn fixture_value(key: &str, raw: &str) -> OwnedVariant {
        match key {
            "Type" => OwnedVariant::from(raw.parse::<u32>().expect("Type is a u32")),
            "Percentage" => OwnedVariant::from(raw.parse::<f64>().expect("Percentage is a double")),
            "IsPresent" => OwnedVariant::from(raw.parse::<bool>().expect("IsPresent is a bool")),
            _ => text(raw),
        }
    }

    fn expect_snapshot(block: &HashMap<String, OwnedVariant>, path: &str) -> DeviceSnapshot {
        snapshot_from_properties(path, block)
            .expect("fixture must parse")
            .expect("fixture device must be a peripheral")
    }

    #[test]
    fn laptop_battery_is_filtered_out() {
        let battery = &fixture_blocks()[0];

        assert_eq!(
            snapshot_from_properties("/org/freedesktop/UPower/devices/battery_BAT1", battery),
            Ok(None)
        );
    }

    #[test]
    fn mouse_becomes_a_device_with_a_level() {
        let snapshot = expect_snapshot(
            &fixture_blocks()[1],
            "/org/freedesktop/UPower/devices/mouse_1",
        );
        let device = snapshot.to_device().expect("mouse is present");

        assert_eq!(device.id, "/org/freedesktop/UPower/devices/mouse_1");
        assert_eq!(device.name, "MX Master 3S");
        assert_eq!(device.kind, DeviceKind::Mouse);
        assert_eq!(device.protocol, Protocol::Upower);
        assert_eq!(snapshot.level(), Some(55));
    }

    #[test]
    fn absent_device_is_not_reported() {
        let mut block = fixture_blocks()[1].clone();
        block.insert("IsPresent".to_owned(), OwnedVariant::from(false));

        let snapshot = expect_snapshot(&block, "/dev/mouse");
        assert_eq!(snapshot.to_device(), None);
    }

    #[test]
    fn empty_model_falls_back_to_the_kind_name() {
        let mut block = fixture_blocks()[1].clone();
        block.insert("Model".to_owned(), text("  "));

        let snapshot = expect_snapshot(&block, "/dev/mouse");
        assert_eq!(snapshot.to_device().expect("present").name, "Mouse");
    }

    #[test]
    fn missing_percentage_is_no_level() {
        let mut block = fixture_blocks()[1].clone();
        block.remove("Percentage");

        let snapshot = expect_snapshot(&block, "/dev/mouse");
        assert_eq!(snapshot.percentage, None);
        assert_eq!(snapshot.level(), None);
        assert!(snapshot.to_device().is_some());
    }

    #[test]
    fn missing_type_is_an_error_naming_the_property() {
        let mut block = fixture_blocks()[1].clone();
        block.remove("Type");

        let err = snapshot_from_properties("/dev/mouse", &block).expect_err("Type is required");
        assert!(err.contains("Type"), "unexpected error: {err}");
    }

    #[test]
    fn wrong_property_type_is_an_error_naming_the_property() {
        let mut block = fixture_blocks()[1].clone();
        block.insert("Percentage".to_owned(), text("55%"));

        let err = snapshot_from_properties("/dev/mouse", &block).expect_err("wrong type");
        assert!(err.contains("Percentage"), "unexpected error: {err}");
    }

    #[test]
    fn upower_types_map_to_device_kinds() {
        assert_eq!(kind_for_upower_type(5), Some(DeviceKind::Mouse));
        assert_eq!(kind_for_upower_type(14), Some(DeviceKind::Mouse));
        assert_eq!(kind_for_upower_type(6), Some(DeviceKind::Keyboard));
        assert_eq!(kind_for_upower_type(12), Some(DeviceKind::Other));
        assert_eq!(kind_for_upower_type(19), Some(DeviceKind::Other));
    }

    #[test]
    fn non_peripheral_types_are_ignored() {
        // unknown, line power, battery, UPS, monitor, computer, camera
        for kind in [0, 1, 2, 3, 4, 11, 25] {
            assert_eq!(kind_for_upower_type(kind), None, "type {kind}");
        }
    }

    #[test]
    fn percentage_is_rounded_and_clamped() {
        assert_eq!(percentage_to_u8(55.4), Some(55));
        assert_eq!(percentage_to_u8(55.6), Some(56));
        assert_eq!(percentage_to_u8(0.0), Some(0));
        assert_eq!(percentage_to_u8(100.0), Some(100));
        assert_eq!(percentage_to_u8(120.0), Some(100));
        assert_eq!(percentage_to_u8(-5.0), Some(0));
    }

    #[test]
    fn non_finite_percentage_has_no_level() {
        assert_eq!(percentage_to_u8(f64::NAN), None);
        assert_eq!(percentage_to_u8(f64::INFINITY), None);
        assert_eq!(percentage_to_u8(f64::NEG_INFINITY), None);
    }
}
