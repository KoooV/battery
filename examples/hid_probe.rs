//! Probes HID devices for a vendor battery report.
//!
//! ```sh
//! cargo run --example hid_probe                        # every HID device
//! cargo run --example hid_probe -- 36b0 3554           # only these vendor ids
//! cargo run --example hid_probe -- --quick 3554        # feature reports only
//! cargo run --example hid_probe -- --listen=60 36b0    # raw reports, all nodes
//! ```
//!
//! Read access to `/dev/hidraw*` is required; install
//! `packaging/udev/99-battery-tray-hidraw.rules` first.
//!
//! The probe is read-only: it asks for feature reports and reads input reports,
//! it never writes to the device. Vendor collections are printed in hex so a
//! battery byte, if the firmware has one, can be spotted by changing level.
//!
//! Keep the mouse moving while it runs: an unsolicited report is often the only
//! place where a cheap receiver puts the charge.

use anyhow::Result;
use hidapi::{DeviceInfo, HidApi, HidDevice};

/// How long to listen for unsolicited input reports, per device.
const LISTEN: std::time::Duration = std::time::Duration::from_secs(15);

fn main() -> Result<()> {
    let mut listen = None;
    let mut quick = false;
    let mut filters: Vec<u16> = Vec::new();

    for arg in std::env::args().skip(1) {
        match arg.strip_prefix("--listen=") {
            Some(seconds) => {
                listen = seconds
                    .parse::<u64>()
                    .ok()
                    .map(std::time::Duration::from_secs)
            }
            None if arg == "--quick" => quick = true,
            None => {
                if let Ok(vendor) = u16::from_str_radix(arg.trim_start_matches("0x"), 16) {
                    filters.push(vendor);
                }
            }
        }
    }

    let api = HidApi::new()?;

    if let Some(duration) = listen {
        return listen_nodes(&api, &filters, duration);
    }

    let mut seen = 0;

    for device in api.device_list() {
        if !filters.is_empty() && !filters.contains(&device.vendor_id()) {
            continue;
        }
        seen += 1;
        describe(device);

        // Only vendor collections can carry a proprietary battery report; the
        // standard keyboard and mouse collections were checked in docs/devices.md.
        if device.usage_page() < 0xff00 {
            println!("   standard collection, skipped");
            continue;
        }

        match device.open_device(&api) {
            Ok(mut handle) => {
                probe_feature_reports(&mut handle);

                // `--quick` is for polling loops: feature reports only.
                if !quick {
                    probe_input_reports(&mut handle);
                }
            }
            Err(err) => println!("   cannot open: {err}"),
        }
    }

    if seen == 0 {
        println!("no HID device matched");
    }

    Ok(())
}

/// Prints every non-zero report of every node of the matching vendors, so it is
/// visible which receiver serves which physical device and what the firmware
/// sends on its own (power-on reports are usually where a cheap kit hides the
/// battery level).
fn listen_nodes(api: &HidApi, filters: &[u16], duration: std::time::Duration) -> Result<()> {
    use std::time::Instant;

    let mut opened: Vec<(String, HidDevice)> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for device in api.device_list() {
        if !filters.is_empty() && !filters.contains(&device.vendor_id()) {
            continue;
        }
        let path = format!("{:?}", device.path());
        if !seen.insert(path.clone()) {
            continue;
        }
        match device.open_device(api) {
            Ok(handle) => {
                println!("listening on {path}");
                opened.push((path, handle));
            }
            Err(err) => println!("{path}: cannot open: {err}"),
        }
    }

    let started = Instant::now();
    let deadline = started + duration;
    let mut buffer = [0u8; 64];

    while Instant::now() < deadline {
        for (path, handle) in opened.iter_mut() {
            let Ok(read) = handle.read_timeout(&mut buffer, 20) else {
                continue;
            };
            if read == 0 || buffer[..read].iter().all(|byte| *byte == 0) {
                continue;
            }
            println!(
                "+{:>6}ms {path} -> {}",
                started.elapsed().as_millis(),
                hex(&buffer[..read])
            );
        }
    }

    println!("done");
    Ok(())
}

fn describe(device: &DeviceInfo) {
    println!(
        "== {:04x}:{:04x} iface={} usage={:04x}:{:04x}",
        device.vendor_id(),
        device.product_id(),
        device.interface_number(),
        device.usage_page(),
        device.usage()
    );
    println!("   path={:?}", device.path());

    if let Some(name) = device.product_string() {
        println!("   name={name}");
    }
    if let Some(manufacturer) = device.manufacturer_string() {
        println!("   manufacturer={manufacturer}");
    }
}

/// Asks for feature reports of every plausible id and length.
fn probe_feature_reports(device: &mut HidDevice) {
    for report_id in 0u8..=0x20 {
        for length in [8usize, 32, 64] {
            let mut buffer = vec![0u8; length];
            buffer[0] = report_id;

            // Print zero payloads too: otherwise a successful read of an empty
            // report looks the same as an unsupported report id.
            let Ok(read) = device.get_feature_report(&mut buffer) else {
                continue;
            };
            let payload = &buffer[..read];
            println!(
                "   feature id={report_id:#04x} len={length} -> {}",
                hex(payload)
            );
        }
    }
}

/// Listens for unsolicited input reports for a while.
fn probe_input_reports(device: &mut HidDevice) {
    println!(
        "   listening {:?} for input reports (move the mouse)",
        LISTEN
    );
    let deadline = std::time::Instant::now() + LISTEN;
    let mut buffer = [0u8; 64];

    while std::time::Instant::now() < deadline {
        let Ok(read) = device.read_timeout(&mut buffer, 200) else {
            break;
        };
        if read == 0 || buffer[..read].iter().all(|byte| *byte == 0) {
            continue;
        }
        println!("   input -> {}", hex(&buffer[..read]));
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
