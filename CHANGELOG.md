# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- CLI parsing now rejects unknown or conflicting options, supports `--` for option-like paths, and preserves non-UTF-8 file names on Unix.
- Keyboard release events no longer repeat commands, append filter text, or dismiss notifications.
- Invalid `DLT?` storage magic is now treated as corruption, allowing parser and TCP recovery to continue with the next valid message.
- Verbose DLT arguments with variable name/unit metadata now follow the AUTOSAR field order and decode correctly.
- Stream recovery no longer loses valid storage-header messages after a corrupt raw header declares an incomplete oversized message.
- File open, decompression, and read failures are now reported in the UI instead of silently showing an empty viewer.
- Local `.claude` session metadata is excluded from crates.io packages, and the packaged README now uses a stable demo image URL.
- `Ctrl+b` (page up) works in the File Explorer again — it was being swallowed by the `b` (batch load) binding.
- Mouse capture is no longer enabled. It was never used for anything, and it blocked the terminal's native text selection, so you couldn't copy log text with the mouse.

### Changed

- CI and release actions are pinned to verified commits, use least-privilege read access by default, and receive weekly Dependabot updates.
- Payload text is now decoded only when displayed, searched, or exported, reducing memory use for large captures while retaining the original bytes for hex views.
- File and TCP ingestion now use a bounded queue, and each UI tick processes a bounded batch to keep large or fast streams responsive.
- The log viewer now builds only visible rows, caches sidebar counts, incrementally merges new timestamp-sorted batches, and shares source paths between entries, substantially reducing CPU, allocations, and memory use on large logs.
- Tagged releases must pass format, Clippy, tests, package, MSRV, and tag-version checks before a GitHub Release is created.
- Sample DLT generation is now an explicit ignored test, so the normal test suite does not write into the repository.
- Rewrote the README, and documented previously undocumented behavior: paging keys in the detail view, `Esc` in the explorer, export file naming, and the single-entry limitation when reading `.zip` archives.
- Homebrew tap formula is now updated automatically on tagged releases (see `RELEASING.md` for the one-time token setup).

## [1.1.0] - 2026-07-04

### Added

- **`--version` / `-V` flag** — Print the version and exit.
- **Prebuilt binaries** — Tagged releases now publish binaries for Linux (x86_64 gnu/musl, aarch64 musl), macOS (Intel / Apple Silicon), and Windows, so no Rust toolchain is needed to install.
- **Declared MSRV** — `rust-version = "1.88"` is now declared in `Cargo.toml` and enforced in CI, so unsupported toolchains fail fast with a clear error.

### Changed

- **Much lighter builds** — `zip` and `chrono` are now compiled without unused default features (drops zstd/aes/bzip2/lzma/ppmd and their C toolchain requirements). Dependency count went from 147 to 124 crates, making `cargo install dlt-tui` faster and more reliable.
- **Smaller release binary** — Release profile now enables thin LTO and symbol stripping.
- CI now tests on Linux, macOS, and Windows.

### Fixed

- Panic messages are no longer swallowed by the alternate screen — the terminal is restored before the panic message is printed.

## [1.0.0] - 2026-05-04

### Added

- **Unified multi-file timeline** — Loaded files are merged into a single timestamp-ordered timeline, making it easier to inspect CTX-split logs together.
- **Source file tracking** — Log rows and detail view now show the source file for each message.
- **Source / CTX sidebar** — Log Viewer shows source and context counts alongside the timeline.
- **CTX colorization** — Context IDs are colored consistently in the timeline and sidebar for faster visual tracking.

### Fixed

- **Parser: invalid short LEN** — DLT messages with LEN smaller than the standard header are rejected.
- **Terminal cleanup** — Raw mode and alternate screen are restored if startup fails after terminal setup begins.
- **File loading memory use** — File parsing now streams messages instead of buffering all messages before display.

## [0.3.4] - 2026-03-06

### Added

- **Timestamp delta view** — Press `t` in Log Viewer to toggle the time display between absolute time and the time delta (Δt) from the previous message. Useful for diagnosing timeouts and race conditions.
- **Export filtered logs** — Press `E` in Log Viewer to export currently filtered logs to a text file.

## [0.3.3] - 2026-03-05

### Added

- **Horizontal scrolling** — Use `Left`/`Right` arrow keys to horizontally scroll the Payload column in the Log Viewer. `Shift+Left`/`Shift+Right` scrolls faster (10 columns at a time).

## [0.3.2] - 2026-03-03

### Fixed

- **Parser: Storage Header optional** — `parse_dlt_message` now supports DLT messages without Storage Header (`DLT\x01`), enabling TCP streaming from daemons that send raw standard header messages
- **Parser: `find_next_sync` false positive reduction** — Heuristic now requires UEH bit and LEN ≥ 14, eliminating false sync detection on common ASCII bytes like space (0x20)
- **Filter: Esc preserves previous value** — Pressing Esc during filter input now cancels without clearing the existing filter (previously destroyed the active filter)
- **Filter: APP ID / CTX ID case-insensitive** — APP ID and CTX ID filters now use case-insensitive matching, consistent with text filter behavior
- **UI: LogDetail empty state** — Status bar shows "No matching logs" instead of misleading "Log 1/0" when filter results are empty

### Removed

- Removed dead `on_enter()` public method that bypassed file loading logic (`handle_key` already handles Enter correctly)

## [0.3.1] - 2026-02-27

### Added

- **Page scrolling** — `Ctrl+f` / `PageDown` (full page down), `Ctrl+b` / `PageUp` (full page up), `Ctrl+d` (half page down), `Ctrl+u` (half page up) for vim-style fast navigation
- Comprehensive filter scenario tests (text, APP ID, CTX ID, log level, compound, regex)
- Strengthened UI rendering tests with actual content verification
- TCP stream tests for garbage recovery, truncation, and receiver-dropped edge cases

### Fixed

- **Security: TCP connect timeout** — Added 5-second connection timeout; unreachable hosts no longer hang indefinitely
- **Security: Verbose decoder bounds check** — VARI name/unit length fields are now validated to prevent out-of-bounds reads from crafted DLT payloads
- **Security: CLI before raw mode** — `--help` and argument errors no longer corrupt the terminal
- **Explorer sort order** — `..` (parent directory) now always appears first regardless of other entry names

### Changed

- Extracted `App::handle_key()` from `run_app()` — key handling is now unit-testable (main.rs reduced from 170+ to 20 lines)
- Refactored verbose payload decoder with shared endian-aware helpers (`read_u16_at`, `read_u32_at`, etc.) — 333→160 lines
- Eliminated per-row `String::clone()` in LogViewer rendering for better performance
- Updated status bar hints with page scrolling keybindings
- Updated README keybinding tables

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

[1.0.0]: https://github.com/tkmsikd/dlt-tui/compare/v0.4.1...v1.0.0
[0.3.4]: https://github.com/tkmsikd/dlt-tui/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/tkmsikd/dlt-tui/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/tkmsikd/dlt-tui/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/tkmsikd/dlt-tui/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/tkmsikd/dlt-tui/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/tkmsikd/dlt-tui/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tkmsikd/dlt-tui/releases/tag/v0.1.0
