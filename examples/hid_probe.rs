//! Probes HID devices for a vendor battery report.
//!
//! ```sh
//! cargo run --example hid_probe                        # every HID device
//! cargo run --example hid_probe -- 36b0 3554           # only these vendor ids
//! cargo run --example hid_probe -- --quick 3554        # feature reports only
//! cargo run --example hid_probe -- --listen=60 36b0    # raw reports, all nodes
//! cargo run --example hid_probe -- --battery=30 3554   # Compx battery query
//! ```
//!
//! Read access to `/dev/hidraw*` is required; install
//! `packaging/udev/99-battery-tray-hidraw.rules` first.
//!
//! Apart from `--battery`, the probe only reads: it asks for feature reports and
//! listens for input reports. `--battery` sends a single 17-byte vendor frame on
//! usage page `0xFF02` — the documented charge query of the Compx/Nordic receiver
//! family — and prints whatever comes back.
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
    let mut battery = false;
    let mut scan = false;
    let mut battery_for = None;
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
            None if arg == "--battery" => battery = true,
            None if arg == "--scan" => scan = true,
            None if arg.starts_with("--battery=") => {
                battery = true;
                battery_for = arg
                    .trim_start_matches("--battery=")
                    .parse::<u64>()
                    .ok()
                    .map(std::time::Duration::from_secs);
            }
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

    if scan {
        return scan_commands(&api, &filters);
    }

    if battery {
        return query_battery(
            &api,
            &filters,
            battery_for.unwrap_or(std::time::Duration::from_secs(10)),
        );
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

/// Sends every read-only command of the family with a few length values and
/// prints the answers, so an unknown firmware can be mapped without guessing.
/// `0x08` is skipped on purpose: it writes EEPROM.
fn scan_commands(api: &HidApi, filters: &[u16]) -> Result<()> {
    use std::time::{Duration, Instant};

    let mut buffer = [0u8; 64];

    for device in api.device_list() {
        if !filters.is_empty() && !filters.contains(&device.vendor_id()) {
            continue;
        }
        if device.usage_page() < 0xff00 {
            continue;
        }
        describe(device);

        let handle = match device.open_device(api) {
            Ok(handle) => handle,
            Err(err) => {
                println!("   cannot open: {err}");
                continue;
            }
        };

        for command in 0x00u8..=0x0b {
            if command == 0x08 {
                continue;
            }
            for length in [0u8, 1, 2, 10] {
                let mut request = frame(command);
                request[5] = length;
                let sum = request[..16]
                    .iter()
                    .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
                request[16] = 0x55u8.wrapping_sub(sum);

                if handle.write(&request).is_err() {
                    continue;
                }
                let deadline = Instant::now() + Duration::from_millis(120);
                while Instant::now() < deadline {
                    let Ok(read) = handle.read_timeout(&mut buffer, 20) else {
                        break;
                    };
                    if read == 0 || buffer[..read].iter().all(|byte| *byte == 0) {
                        continue;
                    }
                    println!(
                        "   cmd={command:#04x} len={length} -> {}",
                        hex(&buffer[..read])
                    );
                }
            }
        }
    }

    Ok(())
}

/// Builds a 17-byte vendor frame: `[08][cmd][00][addr_hi][addr_lo][len][data×10][0x55 − Σ]`.
fn frame(command: u8) -> [u8; 17] {
    let mut frame = [0u8; 17];
    frame[0] = 0x08;
    frame[1] = command;
    let sum = frame[..16]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    frame[16] = 0x55u8.wrapping_sub(sum);
    frame
}

/// Asks the Compx/Nordic receivers for the charge level.
///
/// Command `0x04` on report `0x08` is the documented battery query: the reply
/// carries the percentage at offset 6, a charging flag at 7 and millivolts at
/// 8–9. Some models answer an output report, others a feature report, so both are
/// sent; the command is harmless and changes no configuration.
fn query_battery(api: &HidApi, filters: &[u16], duration: std::time::Duration) -> Result<()> {
    use std::time::{Duration, Instant};

    println!("request {}", hex(&frame(0x04)));

    let mut tried = 0;
    let mut buffer = [0u8; 64];

    for device in api.device_list() {
        if !filters.is_empty() && !filters.contains(&device.vendor_id()) {
            continue;
        }
        // The battery collection is pinned to usage page 0xff02, usage 0x02;
        // every other node of the same interface answers the same way.
        if device.usage_page() != 0xff02 || device.usage() != 0x02 {
            continue;
        }
        tried += 1;
        describe(device);

        let handle = match device.open_device(api) {
            Ok(handle) => handle,
            Err(err) => {
                println!("   cannot open: {err}");
                continue;
            }
        };

        // The receiver answers only while the mouse is awake, so the query is
        // repeated: move the mouse (or click it) during this window.
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            let request = frame(0x04);
            if let Err(err) = handle.write(&request) {
                println!("   output report rejected: {err}");
            }
            let _ = handle.send_feature_report(&request);

            let reply_deadline = Instant::now() + Duration::from_millis(250);
            while Instant::now() < reply_deadline {
                let Ok(read) = handle.read_timeout(&mut buffer, 50) else {
                    break;
                };
                if read == 0 || buffer[..read].iter().all(|byte| *byte == 0) {
                    continue;
                }
                let reply = &buffer[..read];
                println!("   reply -> {}", hex(reply));

                if reply.len() >= 10 && reply[0] == 0x08 && reply[1] == 0x04 {
                    let level = reply[6];
                    let charging = reply[7] != 0;
                    let millivolts = u16::from_be_bytes([reply[8], reply[9]]);
                    println!("   battery: {level}% charging={charging} {millivolts} mV");
                }
            }
        }
    }

    if tried == 0 {
        println!("no vendor collection matched");
    }

    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
