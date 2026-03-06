# dlt-tui

[![Crates.io](https://img.shields.io/crates/v/dlt-tui.svg)](https://crates.io/crates/dlt-tui)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**A fast, keyboard-centric terminal viewer for Automotive DLT (Diagnostic Log and Trace) files.**

Analyze AUTOSAR DLT logs directly in your terminal — no GUI needed. Works over SSH on test benches, in CI pipelines, and inside Docker containers.

<p align="center">
  <img src="assets/demo.png" alt="dlt-tui demo screenshot" width="700">
</p>

---

## Why dlt-tui?

| Pain Point                             | dlt-tui Solution                                            |
| -------------------------------------- | ----------------------------------------------------------- |
| dlt-viewer requires a desktop GUI      | Works in any terminal — SSH, CI runners, containers         |
| Opening multi-GB DLT files is slow     | Async streaming parser — starts displaying before full load |
| Finding the right log is tedious       | Instant regex search + compound filters (Level × APP × CTX) |
| Compressed logs need manual extraction | Transparently reads `.dlt`, `.dlt.gz`, and `.dlt.zip`       |
| Mouse-heavy workflows slow you down    | Vim-style navigation — hands never leave the keyboard       |

## Features

- **Built-in File Explorer** — Browse directories and open files without leaving the TUI
- **Log Table View** — ECU ID, APP ID, CTX ID, Log Level, Timestamp, and Payload at a glance
- **Log Detail and Hex Dump** — Inspect raw payload bytes for deep protocol analysis
- **Color-coded Log Levels** — Fatal (red), Error (light red), Warn (yellow), Info (green), Debug (blue), Verbose (gray)
- **Real-time Filtering** — Stack multiple filters to isolate exactly what you need:
  - `/` — Regex text search across payloads
  - `l` — Filter by minimum log level
  - `a` — Filter by APP ID
  - `c` — Filter by CTX ID
  - `C` — Clear all filters instantly
- **Live TCP Connection** — Connect directly to a running dlt-daemon for real-time log streaming
- **Compression Support** — Directly open `.gz` and `.zip` compressed DLT files
- **Security Hardened** — Zip bomb protection (500MB limit), terminal injection sanitization

## Quick Start

### Install from crates.io

```bash
cargo install dlt-tui
```

### Or build from source

```bash
git clone https://github.com/tkmsikd/dlt-tui.git
cd dlt-tui
cargo build --release
```

### Run

```bash
# Open file explorer in current directory
dlt-tui

# Open a specific directory
dlt-tui /path/to/log/directory/

# Directly open one or multiple DLT files
dlt-tui /path/to/log1.dlt /path/to/log2.dlt.gz

# Connect to a running dlt-daemon over TCP
dlt-tui --connect localhost:3490

# Typical ADB workflow for IVI development
adb forward tcp:3490 tcp:3490
dlt-tui --connect localhost:3490
```

## Keybindings

### File Explorer

| Key                    | Action                      |
| ---------------------- | --------------------------- |
| `j` / `Down`           | Move down                   |
| `k` / `Up`             | Move up                     |
| `Ctrl+f` / `Page Down` | Page down                   |
| `Ctrl+b` / `Page Up`   | Page up                     |
| `Ctrl+d`               | Half page down              |
| `Ctrl+u`               | Half page up                |
| `g` / `Home`           | Jump to top                 |
| `G` / `End`            | Jump to bottom              |
| `Enter`                | Open directory / Load file  |
| `b`                    | Batch load all files in dir |
| `q`                    | Quit                        |

### Log Viewer

| Key                    | Action                                 |
| ---------------------- | -------------------------------------- |
| `j` / `Down`           | Scroll down                            |
| `k` / `Up`             | Scroll up                              |
| `Ctrl+f` / `Page Down` | Page down                              |
| `Ctrl+b` / `Page Up`   | Page up                                |
| `Ctrl+d`               | Half page down                         |
| `Ctrl+u`               | Half page up                           |
| `Left` / `Right`       | Scroll payload horizontally            |
| `Shift+Left`/`Right`   | Scroll payload horizontally fast       |
| `g` / `Home`           | Jump to first log                      |
| `G` / `End`            | Jump to last log                       |
| `Enter`                | Open detail view with hex dump         |
| `/`                    | Search text (regex supported)          |
| `l`                    | Filter by log level (F/E/W/I/D/V)      |
| `a`                    | Filter by APP ID                       |
| `c`                    | Filter by CTX ID                       |
| `C`                    | Clear all filters                      |
| `S`                    | Save filter block to `.dlt-tui.toml`   |
| `L`                    | Load filter block from `.dlt-tui.toml` |
| `F`                    | Toggle auto-scroll (tail mode)         |
| `t`                    | Toggle delta time ($\Delta$t)          |
| `E`                    | Export filtered logs to file           |
| `q` / `Esc`            | Back to File Explorer                  |

### Log Detail

| Key         | Action                       |
| ----------- | ---------------------------- |
| `j` / `k`   | Navigate between log entries |
| `q` / `Esc` | Back to Log Viewer           |

In any filter input mode, press `Enter` to apply or `Esc` to cancel and reset the filter.

## Use Cases

### ECU Bring-Up and Debugging

SSH into your target hardware and inspect DLT logs on the spot — no need to copy files back to your workstation.

```bash
ssh ecu-bench "cat /var/log/dlt/*.dlt" > combined.dlt && dlt-tui combined.dlt
```

### CI / Test Bench Pipeline

Integrate log inspection into your CI pipeline. When a test fails, quickly triage the issue:

```bash
dlt-tui ./test-results/ecu_log_$(date +%Y%m%d).dlt.gz
```

### Quick Triage with Compound Filters

Stack filters to isolate exactly what you need:

1. Press `l`, type `W`, press `Enter` (show warnings and above only)
2. Press `a`, type `DIAG`, press `Enter` (narrow to diagnostics module)
3. Press `/`, type `CAN`, press `Enter` (find CAN-related messages)
4. Press `Enter` on a suspicious log to inspect the hex dump

## Roadmap

- [x] Page-up / Page-down scrolling
- [x] Horizontal scroll for long payloads
- [ ] Bookmarking and log annotation
- [x] Saved filter configurations (`.dlt-tui.toml`)
- [x] Multi-file / directory batch loading
- [x] Timestamp delta display between messages
- [ ] DLT lifecycle and session tracking
- [x] Export filtered logs to file
- [ ] Plugin system for custom decoders (SOME/IP, UDS, etc.)

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

```bash
git clone https://github.com/tkmsikd/dlt-tui.git
cd dlt-tui
cargo test
```

## License

This project is licensed under the [MIT License](LICENSE).
