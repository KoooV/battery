//! Battery data sources.
//!
//! A source owns one protocol (UPower over D-Bus, Logitech HID++, OpenRazer,
//! ...). It enumerates the devices it can see and reads their charge level.
//! Sources are polled from a single loop, so an implementation must not block
//! indefinitely: every bus or device call needs a timeout, and a missing daemon
//! or unplugged device is reported as an error or an empty list, never as a
//! panic.

use thiserror::Error;

/// Kind of peripheral a source reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// Mouse, trackball, trackpad used as a pointing device.
    Mouse,
    /// Keyboard or keypad.
    Keyboard,
    /// Anything else that still carries a battery, e.g. a headset.
    Other,
}

/// Protocol used to talk to a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// Upower exposes the device as a power supply over D-Bus.
    Upower,
    /// Logitech HID++ 2.0 over a Bolt or Unifying receiver.
    Hidpp,
    /// Razer device driven through the OpenRazer daemon.
    OpenRazer,
    /// Protocol that does not fit the named backends yet.
    Other,
}

/// A peripheral whose charge level can be queried.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Device {
    /// Identifier that is stable within its source, e.g. a UPower object path.
    pub id: String,
    /// Name shown in the tray menu and in notifications.
    pub name: String,
    /// What the device is.
    pub kind: DeviceKind,
    /// How the device is reached.
    pub protocol: Protocol,
}

impl Device {
    /// Builds a device description.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        kind: DeviceKind,
        protocol: Protocol,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            protocol,
        }
    }
}

/// Errors produced by [`BatterySource`] implementations.
#[derive(Debug, Error)]
pub enum SourceError {
    /// The backend as a whole is unusable: no daemon, no adapter, no permission.
    // The field is called `backend` and not `source`: `thiserror` treats a
    // field named `source` as a nested error.
    #[error("{backend} is unavailable: {reason}")]
    Unavailable {
        /// Name of the backend that is unavailable.
        backend: &'static str,
        /// Why it cannot be used.
        reason: String,
    },
    /// The backend works, but this one device could not be read.
    #[error("failed to read {device}: {reason}")]
    Read {
        /// Device that failed.
        device: String,
        /// Why the read failed.
        reason: String,
    },
}

/// A provider of battery levels.
pub trait BatterySource: Send + Sync {
    /// Short name of the backend, used in logs and in the tray menu.
    fn name(&self) -> &str;

    /// Lists the devices that currently exist and are visible to the backend.
    ///
    /// An empty vector means "this backend sees nothing right now", which is not
    /// an error: a receiver can be unplugged and every device with it.
    fn list(&self) -> Result<Vec<Device>, SourceError>;

    /// Reads the charge level of one device as a percentage.
    ///
    /// `Ok(None)` means the device is known but says nothing about its charge,
    /// for example because it is asleep behind its receiver.
    fn level(&self, dev: &Device) -> Result<Option<u8>, SourceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal source that serves canned data, the shape the polling loop tests
    /// rely on from stage 4 on.
    struct FixedSource {
        devices: Vec<Device>,
        level: Option<u8>,
    }

    impl BatterySource for FixedSource {
        fn name(&self) -> &str {
            "fixed"
        }

        fn list(&self) -> Result<Vec<Device>, SourceError> {
            Ok(self.devices.clone())
        }

        fn level(&self, _dev: &Device) -> Result<Option<u8>, SourceError> {
            Ok(self.level)
        }
    }

    #[test]
    fn source_trait_is_usable_as_a_trait_object() {
        let source: Box<dyn BatterySource> = Box::new(FixedSource {
            devices: vec![Device::new(
                "mouse0",
                "Test Mouse",
                DeviceKind::Mouse,
                Protocol::Other,
            )],
            level: Some(42),
        });

        let devices = source.list().expect("list must succeed");
        assert_eq!(devices.len(), 1);
        assert_eq!(source.name(), "fixed");
        assert_eq!(
            source.level(&devices[0]).expect("level must succeed"),
            Some(42)
        );
    }

    #[test]
    fn device_new_keeps_all_fields() {
        let device = Device::new("id-1", "Keyboard", DeviceKind::Keyboard, Protocol::Hidpp);

        assert_eq!(device.id, "id-1");
        assert_eq!(device.name, "Keyboard");
        assert_eq!(device.kind, DeviceKind::Keyboard);
        assert_eq!(device.protocol, Protocol::Hidpp);
    }

    #[test]
    fn unavailable_error_names_the_backend() {
        let err = SourceError::Unavailable {
            backend: "upower",
            reason: "no system bus".to_owned(),
        };

        assert_eq!(err.to_string(), "upower is unavailable: no system bus");
    }
}
