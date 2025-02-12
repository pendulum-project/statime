# Changelog

## [0.3.0] - 2025-02-12

### Added
- Hardware clocks can now be determined automatically (enabled by default) in statime-linux
- The instance-level `slave-only` configuration option was added to statime-linux
- Add configuration of `minor-ptp-version` (either 0 or 1), to work around limitations on some PTP networks

### Changed
- Documentation for the observability configuration and CLI command

### Fixed
- Correctly package the metrics daemon, a systemd unit file was also added (disabled by default)

## [0.2.2] - 2024-09-20

### Added
- Added support for the path trace option
- Added support for disabling synchronization of the system clock, touching only the ptp hardware clocks.

### Changed
- Updated dependencies
- Be less chatty about unexpected PTPv1 messages

### Fixed
- Correctly ignore rogue masters in the PTP network

## [0.2.1] - 2024-06-07

### Added
- Threat model in the documentation
- Sample config for IEC/IEEE 61580

### Changed
- Wrap PTP instance's state in a generic mutex and handle announce messages on slave ports
- Handling of multiple ports of the instance being connected to the same network
- Now using tracing instead of fern for logging

### Fixed
- Actually forward TLVs on announce messages
- Fixed two bugs in the BMCA

## [0.2.0] - 2024-03-07

### Added
- Take into account delay asymmetry
- Metrics exporter
- Implement forwaring of TLVs
- Support for peer delay
- udev rules for better permissions

### Changed
- Updated dependencies
- Implement kalman filter for incoming timestamps
- Simplified state management of ports

### Fixed
- Fixed race condition during startup

[0.3.0]: https://github.com/pendulum-project/statime/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/pendulum-project/statime/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/pendulum-project/statime/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/pendulum-project/statime/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/pendulum-project/statime/releases/tag/v0.1.0
