# Devices and data sources

Survey performed for stage 0 of `PLAN.md` on the development machine
(CachyOS, kernel 7.3.4-1-cachyos, UPower 1.91.5). Raw command output lives in
[`upower-dump.txt`](upower-dump.txt) and [`sysfs-dump.txt`](sysfs-dump.txt);
serial numbers are redacted in the committed copies.

## What UPower reports

UPower knows three objects, none of them a peripheral:

| UPower object | Kind | Relevance |
| --- | --- | --- |
| `battery_BAT1` | Laptop battery (Desay HB4792Z9ECW-22T, 37%) | Ignored by design — stage 2 filters out the system battery |
| `line_power_ACAD` | AC adapter | Ignored |
| `DisplayDevice` | Composite display battery | Ignored |

`/sys/class/power_supply/` contains exactly `ACAD` and `BAT1`, so no peripheral
battery is exposed through the kernel either.

**Conclusion:** UPower provides no mouse or keyboard battery on this machine.
Stage 2 will therefore print an empty device list here; the feature is still
worth having for other hardware and other users.

## Peripheral inventory

Taken from `/sys/class/hidraw/*/device/uevent`, `lsusb` and
`/proc/bus/input/devices`.

| Device | VID:PID | Interfaces | UPower sees it | Kernel `power_supply` | Source / verdict |
| --- | --- | --- | --- | --- | --- |
| Keyboard cidoo (wireless, "RDMCTMZT Wireless 2.4G Dongle") | `36b0:3002` | `hidraw0` keyboard, `hidraw2` mouse/system/consumer, `hidraw1` vendor page `0xff60` usage `0x61`, 32-byte input + 32-byte output | no | no | Vendor channel silent, no feature reports. No battery exposed |
| Mouse ardor ("Compx 2.4G Wireless Receiver") | `3554:f53c` | `hidraw3` keyboard/system, `hidraw5` mouse/consumer, `hidraw4` vendor pages `0xff02`, `0xff03`, `0xff04` (feature), `0xff05` | no | no | Feature reports work, but none carries a charge level |
| Touchpad ("GXTP7863:00") | `27c6:01e0` (I2C HID) | `hidraw6` via `hid-multitouch` | no | no | No battery; excluded from all sources |

Other USB devices present are unrelated: webcam (`0408:1040`), Bluetooth
(`0489:e0e1`), fingerprint reader (`27c6:5125`).

No Logitech receiver (Bolt/Unifying) and no Razer device is attached, so stages 6
and 7 cannot be verified on this hardware as is.

## Report descriptors

None of the descriptors declares the standard battery usage (Generic Device
Controls, usage page `0x06` usage `0x20` "Battery Strength"). A generic HID
battery reader would return nothing for these devices; the charge level, if the
firmware reports one at all, is only available in vendor collections
(`0xff00`–`0xff60`), which differ per vendor and are undocumented.

## Access to the HID devices

The kernel creates `/dev/hidraw*` as `crw------- root root`, so neither receiver
can be read by a desktop user out of the box.
[`packaging/udev/99-battery-tray-hidraw.rules`](../packaging/udev/99-battery-tray-hidraw.rules)
grants access to both receivers with `GROUP="users"` and `MODE="0660"`.
`TAG+="uaccess"` alone produced no ACL on this machine — `getfacl` still showed
only `user::rw-`, `group::rw-`, `other::---` and opening the node failed with
`Permission denied` — so the group is set explicitly. This is needed by any
vendor backend; stages 1–5 do not touch hidraw.

## Vendor protocol probe

Measured on 2026-11-18 with `examples/hid_probe.rs`, which is read-only: it asks
for feature reports and listens for unsolicited input reports, it never writes to
the firmware.

The physical pairing is not what the interface layout suggests:

| Physical device | Receiver | Evidence |
| --- | --- | --- |
| cidoo keyboard | `36b0:3002` | typing produced key reports on `hidraw0` |
| ardor mouse | `3554:f53c` | mouse motion produced reports on `hidraw5` |

### `36b0:3002` (RDMCTMZT)

- `hidraw1`, vendor page `0xff60` usage `0x61`, 32-byte input and 32-byte output:
  **zero reports** during five minutes of active keyboard use.
- Every feature report id `0x00`–`0x20` fails, so the firmware cannot be queried
  through the standard feature-report ioctl at all.
- `hidraw2` produced `06 …` (32 bytes), `04 00 00`, `02 00 00 00 00 00` and
  `03 00 00`, twice, with every field zero.

### `3554:f53c` (Compx)

- Feature reports exist for ids `0x00`, `0x02`, `0x03`, `0x05`, `0x06`, `0x08`
  and nothing above `0x08`.
- `0x06` answers `06 00 00 64 64 64 9b 97 c9` (9 bytes); the payload stayed
  byte-for-byte identical between reads minutes apart. All other ids read zero.
- One vendor report per power event of the device:
  `08 0a 00 00 00 0a 40 00 00 00 00 00 00 00 00 00 f9`, twice, identical both
  times.

### Verdict

Neither receiver exposes a battery level. There is no standard HID battery page,
no queryable feature report carrying a percentage, and no unsolicited report that
changes with the device state. The only unexplained byte is `0x40` (64) in the
17-byte power-event report; three `100`s in feature report `0x06` are also
indicative-looking. Both are unproven and, given that the whole packets repeat
identically across power cycles, more consistent with a fixed configuration blob
(DPI stages, report timing) than with a live charge reading.

## Open items

- To settle the `0x40` / `100,100,100` candidates one needs a second data point at
  a different charge: repeat the probe with a device that is visibly not at 100 %.
  Until then no backend is written for these receivers — guessing a vendor
  protocol would produce wrong icons in the tray.
- If the user has Logitech or Razer hardware, attach it once so the HID++ and
  OpenRazer backends can be tested.
