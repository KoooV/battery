//! Writes sample tray icons as PNG files so they can be checked by eye.
//!
//! ```sh
//! cargo run --example preview_icons -- target/icons
//! ```
//!
//! The directory defaults to `target/icons`. The PNGs are build output, not
//! source files.

use std::path::PathBuf;

use anyhow::{Context, Result};
use battery_tray::icon;

fn main() -> Result<()> {
    let directory = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/icons"), PathBuf::from);
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("cannot create {}", directory.display()))?;

    for level in [
        Some(100),
        Some(55),
        Some(31),
        Some(30),
        Some(10),
        Some(5),
        None,
    ] {
        let pixmap = icon::render(level).context("cannot render the icon")?;
        let name = level.map_or_else(|| "unknown".to_owned(), |level| level.to_string());
        let path = directory.join(format!("{name}.png"));

        pixmap
            .save_png(&path)
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!("{}", path.display());
    }

    Ok(())
}
