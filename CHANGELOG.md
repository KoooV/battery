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
- Stage 0 device survey in `docs/`: what UPower and the kernel expose on the
  development machine and which backend each device would need.
