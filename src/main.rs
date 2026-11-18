//! `battery-tray` — shows battery levels of mice and keyboards in the system tray.

use anyhow::Result;
use tracing::info;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("battery-tray {} starting", env!("CARGO_PKG_VERSION"));

    // Polling loop, tray item and battery sources are added in the next stages
    // of PLAN.md; the binary only sets up logging for now.
    Ok(())
}
