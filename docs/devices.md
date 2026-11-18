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
| Mouse + keyboard (wireless, "RDMCTMZT Wireless 2.4G Dongle") | `36b0:3002` | `hidraw0` keyboard, `hidraw2` mouse/system/consumer, `hidraw1` vendor page `0xff60` usage `0x61`, 32-byte input + 32-byte output | no | no | Vendor-specific HID only, protocol undocumented. Not supported yet |
| Keyboard (same 2.4G dongle) | `36b0:3002` | `hidraw0` | no | no | Same dongle as above, same vendor channel. Not supported yet |
| Mouse + keyboard ("Compx 2.4G Wireless Receiver") | `3554:f53c` | `hidraw3` keyboard/system, `hidraw5` mouse/consumer, `hidraw4` vendor pages `0xff02`, `0xff03`, `0xff04` (feature), `0xff05` | no | no | Vendor-specific HID only, protocol undocumented. Not supported yet |
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

`/dev/hidraw*` here is `crw------- root root`, and the current user is not in a
group that would grant access. Reading HID++ or vendor reports therefore needs
either a `udev` rule (a `plugdev`/`uaccess` tag on the receiver's hidraw nodes)
or elevated privileges. That has to be solved before stage 6; it is not needed
for stages 1–5.

## Open items

- Vendor battery protocol for `36b0:3002` and `3554:f53c` is unknown. Checking
  whether the dongles answer a battery query needs a hidraw read/write test with
  the udev rule above, which requires the owner's permission.
- If the user has Logitech or Razer hardware, attach it once so the HID++ and
  OpenRazer backends can be tested.
