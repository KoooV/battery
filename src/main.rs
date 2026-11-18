//! `battery-tray` — shows battery levels of mice and keyboards in the system tray.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use battery_tray::tray::DeviceTray;
use battery_tray::{BatterySource, UpowerSource};
use ksni::blocking::TrayMethods;
use tracing::{info, warn};

/// How often the main thread checks whether the user asked to quit. Polling the
/// sources is added in stage 4.
const QUIT_CHECK_INTERVAL: Duration = Duration::from_millis(200);

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("battery-tray {} starting", env!("CARGO_PKG_VERSION"));

    let quit = Arc::new(AtomicBool::new(false));

    // The level is read once: the polling loop that keeps it fresh is stage 4.
    let devices = match UpowerSource::connect() {
        Ok(source) => match source.list() {
            Ok(devices) => devices
                .into_iter()
                .map(|device| {
                    let level = match source.level(&device) {
                        Ok(level) => level,
                        Err(err) => {
                            warn!("{err}");
                            None
                        }
                    };
                    (device, level)
                })
                .collect::<Vec<_>>(),
            Err(err) => {
                warn!("{err}");
                Vec::new()
            }
        },
        Err(err) => {
            warn!("{err}");
            Vec::new()
        }
    };

    if devices.is_empty() {
        warn!("no device with a battery found, nothing to show in the tray");
        return Ok(());
    }

    let mut handles = Vec::new();
    for (device, level) in devices {
        info!(
            "{}: {}",
            device.name,
            level.map_or("no data".to_owned(), |level| format!("{level}%"))
        );

        match DeviceTray::new(device, level, Arc::clone(&quit)).spawn() {
            Ok(handle) => handles.push(handle),
            Err(err) => warn!("cannot add a tray icon: {err}"),
        }
    }

    if handles.is_empty() {
        warn!("no tray icon could be added, exiting");
        return Ok(());
    }

    while !quit.load(Ordering::Relaxed) {
        thread::sleep(QUIT_CHECK_INTERVAL);
    }

    for handle in handles {
        handle.shutdown().wait();
    }

    info!("quit requested from the tray, exiting");
    Ok(())
}
