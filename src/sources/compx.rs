//! Compx / Nordic 2.4G receiver battery source.
//!
//! Cheap 2.4G mice (Compx, ATK, VXE, VGN, Zaopin, Ardor and many rebrands) hang
//! off a receiver that splits its vendor interface into several HID collections.
//! One of them — usage page `0xff02`, usage `0x02` — carries a 17-byte
//! request/reply protocol:
//!
//! ```text
//! write  08 04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 49
//! reply  08 04 00 00 00 02 64 00 11 2d 00 00 00 00 00 00 a5
//!  ^id ^cmd          ^-  ^%  ^w  ^^^^^ mV          ^checksum
//! ```
//!
//! Byte 0 is the report id, byte 1 the command (`0x04` asks for the charge),
//! byte 2 the result (`0` is success), byte 6 the percentage, byte 7 a
//! wired/charging flag and bytes 8–9 the cell voltage in millivolts,
//! big-endian. The checksum makes all 17 bytes sum to `0x55`.
//!
//! Two properties of this family matter for the polling loop:
//!
//! * The receiver answers only while the device is awake; a mouse that went to
//!   sleep leaves the query unanswered, so a read is a bounded wait of at most
//!   [`REPLY_TIMEOUT`] and then "no data", never a hang.
//! * The firmware does *not* support the standard HID battery page, and feature
//!   report `0x06` on the neighbouring `0xff04` collection reports full-charge
//!   capacity, not the charge — reading it once gives a constant 100%. Only the
//!   `0x04` command above carries the live level, which is why this source
//!   writes one frame instead of reading a report.

use std::collections::HashSet;
use std::ffi::CString;
use std::time::{Duration, Instant};

use hidapi::HidApi;

use super::{BatterySource, Device, DeviceKind, Protocol, SourceError};

/// Name of this backend in logs and errors.
const BACKEND: &str = "compx";

/// Vendor collection that carries the protocol, taken from the report descriptor
/// of the receiver (`06 02 ff`, usage `0x02`).
const VENDOR_USAGE_PAGE: u16 = 0xff02;
/// Usage of that collection.
const VENDOR_USAGE: u16 = 0x0002;

/// Report id of every frame of the family.
const REPORT_ID: u8 = 0x08;
/// Command that asks for the charge level.
const COMMAND_BATTERY: u8 = 0x04;
/// Every frame, request and reply, is 17 bytes long.
const FRAME_LEN: usize = 17;
/// A valid frame sums to this value, so the last byte is `0x55 - sum(16)`.
const FRAME_SUM: u8 = 0x55;

/// Longest wait for a reply. The device is usually awake, so the loop returns as
/// soon as it answers; a sleeping one costs the full timeout and reports no data.
const REPLY_TIMEOUT: Duration = Duration::from_millis(1500);
/// Slice of a wait, small enough to notice an answer quickly.
const REPLY_SLICE: Duration = Duration::from_millis(100);

/// Charge level decoded from a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BatteryReading {
    /// Charge in percent.
    level: u8,
    /// Whether the device reports being on a cable.
    charging: bool,
    /// Cell voltage in millivolts.
    millivolts: u16,
}

/// Builds the request frame for a command.
fn frame(command: u8) -> [u8; FRAME_LEN] {
    let mut frame = [0u8; FRAME_LEN];
    frame[0] = REPORT_ID;
    frame[1] = command;
    frame[FRAME_LEN - 1] = checksum(&frame);
    frame
}

/// Checksum byte that makes the whole frame sum to [`FRAME_SUM`].
fn checksum(frame: &[u8]) -> u8 {
    let sum = frame[..FRAME_LEN - 1]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte));

    FRAME_SUM.wrapping_sub(sum)
}

/// Decodes a reply, returning `None` for anything that is not a successful
/// charge answer: a reply of another command, an error result, a bad length, a
/// broken checksum or an impossible percentage.
fn parse_reply(bytes: &[u8]) -> Option<BatteryReading> {
    if bytes.len() != FRAME_LEN {
        return None;
    }
    if bytes[0] != REPORT_ID || bytes[1] != COMMAND_BATTERY || bytes[2] != 0 {
        return None;
    }
    if bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != FRAME_SUM {
        return None;
    }

    let level = bytes[6];
    if level > 100 {
        return None;
    }

    Some(BatteryReading {
        level,
        charging: bytes[7] != 0,
        millivolts: u16::from_be_bytes([bytes[8], bytes[9]]),
    })
}

/// Name to show for a receiver: its product string, or a generic one.
fn device_name(product: Option<&str>) -> String {
    match product.map(str::trim) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => "2.4G receiver".to_owned(),
    }
}

/// [`BatterySource`] backed by the vendor protocol of Compx-family receivers.
pub struct CompxSource;

impl CompxSource {
    /// Opens the HID subsystem.
    ///
    /// A receiver that is unplugged is not an error: it simply does not appear in
    /// [`BatterySource::list`], so plugging it in later works without a restart.
    pub fn connect() -> Result<Self, SourceError> {
        HidApi::new().map_err(|err| SourceError::Unavailable {
            backend: BACKEND,
            reason: format!("cannot open the HID subsystem: {err}"),
        })?;

        Ok(Self)
    }

    /// Opens one receiver, sends the battery query and waits for the answer.
    fn query(&self, id: &str) -> Result<Option<BatteryReading>, String> {
        let api = HidApi::new().map_err(|err| format!("cannot open the HID subsystem: {err}"))?;
        let path = CString::new(id).map_err(|err| format!("not a device path: {err}"))?;
        let handle = api
            .open_path(&path)
            .map_err(|err| format!("cannot open the receiver: {err}"))?;

        handle
            .write(&frame(COMMAND_BATTERY))
            .map_err(|err| format!("cannot send the battery query: {err}"))?;

        let deadline = Instant::now() + REPLY_TIMEOUT;
        let mut buffer = [0u8; 64];

        while Instant::now() < deadline {
            let read = match handle.read_timeout(&mut buffer, REPLY_SLICE.as_millis() as i32) {
                Ok(read) => read,
                Err(err) => return Err(format!("cannot read the receiver: {err}")),
            };

            if let Some(reading) = parse_reply(&buffer[..read]) {
                return Ok(Some(reading));
            }
        }

        Ok(None)
    }
}

impl BatterySource for CompxSource {
    fn name(&self) -> &str {
        BACKEND
    }

    fn list(&self) -> Result<Vec<Device>, SourceError> {
        let api = HidApi::new().map_err(|err| SourceError::Unavailable {
            backend: BACKEND,
            reason: format!("cannot open the HID subsystem: {err}"),
        })?;

        let mut devices = Vec::new();
        let mut seen = HashSet::new();

        for info in api.device_list() {
            if info.usage_page() != VENDOR_USAGE_PAGE || info.usage() != VENDOR_USAGE {
                continue;
            }

            // The receiver exposes the same node through several usages; one
            // device per node, not per report descriptor entry.
            let path = info.path().to_string_lossy().into_owned();
            if !seen.insert(path.clone()) {
                continue;
            }

            devices.push(Device::new(
                path,
                device_name(info.product_string()),
                DeviceKind::Mouse,
                Protocol::Compx,
            ));
        }

        Ok(devices)
    }

    fn level(&self, dev: &Device) -> Result<Option<u8>, SourceError> {
        let reading = self.query(&dev.id).map_err(|reason| SourceError::Read {
            device: dev.id.clone(),
            reason,
        })?;

        Ok(reading.map(|reading| reading.level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a reply frame the way the firmware would.
    fn reply(level: u8, charging: bool, millivolts: u16) -> [u8; FRAME_LEN] {
        let mut frame = frame(COMMAND_BATTERY);
        frame[5] = 0x02;
        frame[6] = level;
        frame[7] = u8::from(charging);
        let [high, low] = millivolts.to_be_bytes();
        frame[8] = high;
        frame[9] = low;
        frame[FRAME_LEN - 1] = checksum(&frame);

        frame
    }

    #[test]
    fn request_matches_the_documented_frame() {
        assert_eq!(
            frame(COMMAND_BATTERY),
            [
                0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x49
            ]
        );
    }

    #[test]
    fn every_frame_sums_to_the_family_constant() {
        for command in [0x00u8, 0x04, 0x0a, 0xff] {
            let frame = frame(command);
            let sum = frame.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));

            assert_eq!(sum, FRAME_SUM, "command {command:#04x}");
        }
    }

    #[test]
    fn captured_full_receiver_reply_is_parsed() {
        // Captured from a Compx `3554:f53c` with a mouse at full charge.
        let captured = [
            0x08, 0x04, 0x00, 0x00, 0x00, 0x02, 0x64, 0x00, 0x11, 0x2d, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0xa5,
        ];

        assert_eq!(
            parse_reply(&captured),
            Some(BatteryReading {
                level: 100,
                charging: false,
                millivolts: 4397,
            })
        );
    }

    #[test]
    fn partly_discharged_reply_is_parsed() {
        // The layout of the public reference capture, checksum recomputed.
        assert_eq!(
            parse_reply(&reply(35, false, 3732)),
            Some(BatteryReading {
                level: 35,
                charging: false,
                millivolts: 3732,
            })
        );
    }

    #[test]
    fn charging_flag_is_reported() {
        let reading = parse_reply(&reply(80, true, 4100)).expect("valid reply");

        assert!(reading.charging);
    }

    #[test]
    fn error_result_is_not_a_reading() {
        let mut frame = reply(50, false, 3900);
        frame[2] = 0x01;
        frame[FRAME_LEN - 1] = checksum(&frame);

        assert_eq!(parse_reply(&frame), None);
    }

    #[test]
    fn reply_of_another_command_is_ignored() {
        let link_status = [
            0x08, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x9b, 0x97, 0xc9, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x4e,
        ];

        assert_eq!(parse_reply(&link_status), None);
    }

    #[test]
    fn full_capacity_feature_report_is_not_a_reading() {
        // Feature report 0x06: three `0x64` are the full-charge capacity, not the
        // live level. It must never be mistaken for a charge.
        let trap = [0x06, 0x00, 0x00, 0x64, 0x64, 0x64, 0x9b, 0x97, 0xc9];

        assert_eq!(parse_reply(&trap), None);
    }

    #[test]
    fn broken_checksum_is_rejected() {
        let mut frame = reply(50, false, 3900);
        frame[FRAME_LEN - 1] ^= 0xff;

        assert_eq!(parse_reply(&frame), None);
    }

    #[test]
    fn short_or_long_buffers_are_rejected() {
        let frame = reply(50, false, 3900);

        assert_eq!(parse_reply(&[]), None);
        assert_eq!(parse_reply(&frame[..9]), None);

        let mut longer = [0u8; FRAME_LEN + 1];
        longer[..FRAME_LEN].copy_from_slice(&frame);
        assert_eq!(parse_reply(&longer), None);
    }

    #[test]
    fn impossible_percentage_is_rejected() {
        assert_eq!(parse_reply(&reply(101, false, 3900)), None);
        assert_eq!(parse_reply(&reply(0xff, false, 3900)), None);
    }

    #[test]
    fn boundaries_are_accepted() {
        assert_eq!(
            parse_reply(&reply(0, false, 3300)).map(|reading| reading.level),
            Some(0)
        );
        assert_eq!(
            parse_reply(&reply(100, true, 4200)).map(|reading| reading.level),
            Some(100)
        );
    }

    #[test]
    fn name_falls_back_when_the_product_string_is_empty() {
        assert_eq!(
            device_name(Some("2.4G Wireless Receiver")),
            "2.4G Wireless Receiver"
        );
        assert_eq!(device_name(Some("  ")), "2.4G receiver");
        assert_eq!(device_name(None), "2.4G receiver");
    }

    #[test]
    fn backend_name_is_stable() {
        assert_eq!(CompxSource.name(), "compx");
    }
}
