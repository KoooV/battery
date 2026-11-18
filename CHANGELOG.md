# Changelog

All notable changes to this project are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Project skeleton: Rust package `battery-tray` with the planned dependencies
  (`zbus`, `ksni`, `hidapi`, `notify-rust`, `tiny-skia`, `tracing`, `anyhow`,
  `thiserror`) and a `tracing`-based entry point.
- Module skeleton (`sources`, `state`, `tray`, `notify`, `icon`), the `Device`
  model, the `BatterySource` trait, `DeviceState` and the default constants
  (`POLL_INTERVAL` 10 minutes, `LOW_THRESHOLD` 10%).
- UPower source (`UpowerSource`): enumerates devices on the system bus, keeps
  mice, keyboards and other peripherals, drops the laptop battery and line
  power, and reports the charge level of each device. Calls are bounded by a
  5 s timeout and a missing daemon or vanished device is an error, not a crash.
- Unit tests for the UPower mapping and parsing on a saved `GetAll` reply
  (`tests/fixtures/upower-getall.txt`).
- Tray: one StatusNotifierItem per device (ksni), with the device name and
  charge in the menu and a "Quit" entry that stops the application.
- Icon renderer (tiny-skia): rounded background coloured by level (green above
  30%, amber 11–30%, red at 10% and below), the percentage in a built-in 3×5
  bitmap font, and `?` on grey while the level is unknown. `cargo run --example
  preview_icons -- target/icons` writes sample PNGs for a visual check.
- Polling loop: every source is polled at start and then every 10 minutes on
  the main thread, and the tray follows the result — new devices get an icon,
  a gone device loses it, a device that cannot be read stays with "no data".
  `poll` is split into `poll_once` and `sync` and tested with a fake source.
- Temporary demo source for checking the tray without hardware:
  `BATTERY_TRAY_DEMO_LEVEL=42` (or `none`) and `BATTERY_TRAY_DEMO_NAME`.
- Stage 0 device survey in `docs/`: what UPower and the kernel expose on the
  development machine and which backend each device would need.
