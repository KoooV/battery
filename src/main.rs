//! `battery-tray` — shows battery levels of mice and keyboards in the system tray.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use battery_tray::poll;
use battery_tray::sources::demo::{DemoSource, LEVEL_ENV};
use battery_tray::tray::TrayItems;
use battery_tray::{BatterySource, POLL_INTERVAL, UpowerSource};
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

    let quit = Arc::new(AtomicBool::new(false));

    let mut sources: Vec<Box<dyn BatterySource>> = Vec::new();
    match UpowerSource::connect() {
        Ok(source) => sources.push(Box::new(source)),
        Err(err) => warn!("{err}"),
    }

    if let Some(demo) = DemoSource::from_env() {
        info!("demo device enabled through {LEVEL_ENV}");
        sources.push(Box::new(demo));
    }

    if sources.is_empty() {
        warn!("no battery source is available, nothing to poll, exiting");
        return Ok(());
    }

    // Polling runs here, on the main thread; tray callbacks never touch the bus.
    let mut tray = TrayItems::new(Arc::clone(&quit));
    poll::run(&sources, &mut tray, POLL_INTERVAL, &quit);
    tray.shutdown();

    info!("quit requested from the tray, exiting");
    Ok(())
}
