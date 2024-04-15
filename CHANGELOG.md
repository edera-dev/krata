# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.9](https://github.com/edera-dev/krata/compare/v0.0.8...v0.0.9) - 2024-04-15

### Added
- oci compliance work ([#85](https://github.com/edera-dev/krata/pull/85))
- oci packer can now use mksquashfs if available ([#70](https://github.com/edera-dev/krata/pull/70))
- basic kratactl top command ([#72](https://github.com/edera-dev/krata/pull/72))
- idm snooping ([#71](https://github.com/edera-dev/krata/pull/71))
- implement oci image progress ([#64](https://github.com/edera-dev/krata/pull/64))
- guest metrics support ([#46](https://github.com/edera-dev/krata/pull/46))

### Other
- init: default to xterm if TERM is not set ([#52](https://github.com/edera-dev/krata/pull/52))
- update Cargo.toml dependencies

## [0.0.8](https://github.com/edera-dev/krata/compare/v0.0.7...v0.0.8) - 2024-04-09

### Other
- update Cargo.lock dependencies

## [0.0.7](https://github.com/edera-dev/krata/compare/v0.0.6...v0.0.7) - 2024-04-09

### Other
- update Cargo.toml dependencies
- update Cargo.lock dependencies

## [0.0.6](https://github.com/edera-dev/krata/compare/v0.0.5...v0.0.6) - 2024-04-09

### Fixed
- increase channel acquisition timeout to support lower performance hosts ([#36](https://github.com/edera-dev/krata/pull/36))

### Other
- update Cargo.toml dependencies
- update Cargo.lock dependencies

## [0.0.5](https://github.com/edera-dev/krata/compare/v0.0.4...v0.0.5) - 2024-04-09

### Added
- *(ctl)* add help and about to commands and arguments ([#25](https://github.com/edera-dev/krata/pull/25))

### Other
- update Cargo.toml dependencies
- update Cargo.lock dependencies

## [0.0.4](https://github.com/edera-dev/krata/releases/tag/v${version}) - 2024-04-03

### Other
- implement automatic releases
- reimplement console to utilize channels, and provide logs support
- set hostname from launch config
- implement event stream retries
- work on parallel reconciliation
- implement parallel guest reconciliation
- log when a guest start failures occurs
- remove device restriction
- setup loopback interface
- place running tasks in cgroup
