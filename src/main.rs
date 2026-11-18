//! `battery-tray` — shows battery levels of mice and keyboards in the system tray.

use anyhow::Result;
use battery_tray::{BatterySource, UpowerSource};
use tracing::{info, warn};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("battery-tray {} starting", env!("CARGO_PKG_VERSION"));

    // The tray is added in stage 3; until then the polled devices are printed so
    // the UPower source can be checked on a real machine.
    match UpowerSource::connect() {
        Ok(source) => report(&source),
        Err(err) => warn!("{err}"),
    }

    Ok(())
}

/// Prints every device the source knows and its current level.
fn report(source: &UpowerSource) {
    let devices = match source.list() {
        Ok(devices) => devices,
        Err(err) => {
            warn!("{err}");
            return;
        }
    };

    if devices.is_empty() {
        info!("{}: no peripheral with a battery", source.name());
        return;
    }

    for device in devices {
        match source.level(&device) {
            Ok(Some(level)) => info!("{} ({:?}): {level}%", device.name, device.kind),
            Ok(None) => info!("{} ({:?}): no charge reported", device.name, device.kind),
            // A device that vanished between listing and reading is not fatal.
            Err(err) => warn!("{err}"),
        }
    }
}
