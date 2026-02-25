# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.1.0]: https://github.com/tkmsikd/dlt-tui/releases/tag/v0.1.0
