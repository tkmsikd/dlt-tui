# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-02-25

### Added

- **Verbose payload decoder** — DLT verbose mode TLV arguments (string, uint, sint, float, raw) are now decoded into human-readable text instead of showing garbled binary
- **File parser error recovery** — `parse_all_messages` scans for next valid DLT marker on parse errors instead of stopping, recovering all valid messages from corrupted files

### Fixed

- **Standard Header LEN byte order** — Fixed from little-endian to big-endian per AUTOSAR DLT specification; real-world DLT files now parse correctly
- **WEID/WSID/WTMS optional header fields** — ECU ID, Session ID, and Timestamp fields in the standard header are now properly consumed instead of being misinterpreted as payload
- **MSIN bit interpretation** — Fixed to spec: bit 0 = verbose flag, bits 1-3 = MSTP, bits 4-7 = MTIN (was incorrectly treating bit 0 as part of message type)

### Changed

- Status bar shows `[RECOVERED: N bytes skipped]` when file parsing encountered and recovered from corrupted data
- Shared `find_next_sync` logic between TCP client and file parser (DRY refactor)

## [0.2.0] - 2026-02-25

### Added

- **Live TCP connection to dlt-daemon** — `--connect HOST:PORT` for real-time log streaming
- **Auto-scroll (tail) mode** — `F` key to toggle following the latest log in real-time
- **CLI argument parser** — `--connect` / `-c`, `--help` / `-h` flags
- **TCP stream sync recovery** — automatic re-synchronization on corrupted or partial data
- **GitHub Actions CI** — automated test, clippy, and format checks on every push/PR

### Changed

- Status bar now shows `[TCP: addr]` when connected and `[TAIL]` when auto-scroll is active

## [0.1.0] - 2026-02-25

### Added

- **File Explorer** with directory browsing and vim-style navigation
- **DLT Parser** supporting AUTOSAR DLT storage header format
- **Log Viewer** with color-coded log levels (Fatal/Error/Warn/Info/Debug/Verbose)
- **Log Detail screen** with payload metadata and hex dump view
- **Transparent compression** support for `.gz` and `.zip` files
- **Real-time filtering** with regex text search, APP ID, CTX ID, and min log level
- **Async file loading** with streaming parser for large files
- **Hierarchical navigation** — `q` key navigates back through screens before quitting
- **Human-readable timestamps** parsed from DLT storage headers
- **Security hardening** — Zip bomb protection (500MB limit), terminal injection sanitization
- **CLI argument support** — pass a directory or file path to open directly

[0.3.0]: https://github.com/tkmsikd/dlt-tui/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/tkmsikd/dlt-tui/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tkmsikd/dlt-tui/releases/tag/v0.1.0
