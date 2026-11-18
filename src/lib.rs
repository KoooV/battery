//! Core library of `battery-tray`.
//!
//! Every way of reading a charge level is a separate implementation of
//! [`BatterySource`], so the polling loop and the tray stay independent of the
//! protocol a device speaks.

pub mod icon;
pub mod notify;
pub mod sources;
pub mod state;
pub mod tray;

pub use sources::{BatterySource, Device, DeviceKind, Protocol, SourceError, UpowerSource};
pub use state::{DeviceState, LOW_THRESHOLD, POLL_INTERVAL};
