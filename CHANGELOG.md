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
- Stage 0 device survey in `docs/`: what UPower and the kernel expose on the
  development machine and which backend each device would need.
