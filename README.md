# dlt-tui

[![Crates.io](https://img.shields.io/crates/v/dlt-tui.svg)](https://crates.io/crates/dlt-tui)
[![Downloads](https://img.shields.io/crates/d/dlt-tui.svg)](https://crates.io/crates/dlt-tui)
[![CI](https://github.com/tkmsikd/dlt-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/tkmsikd/dlt-tui/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/MSRV-1.88-blue.svg)](https://github.com/tkmsikd/dlt-tui)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A terminal viewer for automotive DLT (Diagnostic Log and Trace) files.

<p align="center">
  <img src="https://raw.githubusercontent.com/tkmsikd/dlt-tui/e6574958afaef2e2c17dca93c91a5a4efb92543b/assets/demo.gif" alt="dlt-tui demo — scrolling, filtering, regex search, hex dump" width="700">
</p>

DLT logs usually live on machines you reach over SSH: test benches, HIL rigs, CI runners. The standard tooling ([dlt-viewer](https://github.com/COVESA/dlt-viewer)) is a Qt desktop app, so in practice you end up copying files back to your workstation just to look at them. dlt-tui is the alternative I wanted: open the file where it is, filter down to the interesting part, and read it — all in the terminal, with vim-style keys.

What it does:

- Opens `.dlt` files, plus `.dlt.gz` and `.dlt.zip` without unpacking first
- Loads multiple files or whole directories, merged into one timestamp-ordered timeline (useful when a session is split across per-context files)
- Filters stack on top of each other: minimum log level, APP ID, CTX ID, and regex search over payloads
- Shows a hex dump of the raw payload for any message
- Streams live from a running dlt-daemon over TCP (`--connect host:port`)
- Exports the currently filtered view to a file
- Parses in a streaming fashion, so large files start displaying before they finish loading

It deliberately does less than dlt-viewer — no ECU configuration, no message injection, no plugins (yet). It's for reading logs, and it tries to be very good at that.

## Install

Homebrew (macOS / Linux):

```bash
brew install tkmsikd/tap/dlt-tui
```

Prebuilt binaries for Linux (x86_64 / aarch64, including static musl builds), macOS, and Windows are on the [releases page](https://github.com/tkmsikd/dlt-tui/releases). The musl builds run on any distro with no dependencies:

```bash
curl -L https://github.com/tkmsikd/dlt-tui/releases/latest/download/dlt-tui-x86_64-unknown-linux-musl.tar.gz | tar xz
./dlt-tui
```

With a Rust toolchain (1.88+): `cargo install dlt-tui`, or clone and `cargo build --release`.

## Usage

```bash
dlt-tui                                  # file explorer in the current directory
dlt-tui /var/log/dlt/                    # ... or a specific directory
dlt-tui boot.dlt session.dlt.gz          # open files as one merged timeline
dlt-tui --connect localhost:3490         # stream from a running dlt-daemon
dlt-tui -- -capture.dlt                  # use -- for paths beginning with '-'
```

For an Android IVI target, forward the daemon port first:

```bash
adb forward tcp:3490 tcp:3490
dlt-tui --connect localhost:3490
```

A typical triage session: press `l` `W` `Enter` to hide everything below warnings, `a` `DIAG` `Enter` to narrow to one application, then `/` with a regex to find the message you're after, and `Enter` on it for the hex dump. `C` clears all filters. `S` saves the current filter stack to `.dlt-tui.toml` so you can reload it with `L` next time.

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
| `q` / `Esc`            | Quit                        |

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
| `t`                    | Toggle delta time between messages     |
| `E`                    | Export filtered logs to file           |
| `q` / `Esc`            | Back to File Explorer                  |

### Log Detail

| Key         | Action                       |
| ----------- | ---------------------------- |
| `j` / `k`   | Navigate between log entries |
| `q` / `Esc` | Back to Log Viewer           |

Paging and jump keys (`Ctrl+f/b/d/u`, `g`, `G`) work in the detail view too. In any filter input, `Enter` applies and `Esc` cancels the input — an already-active filter stays as it was.

## Notes

- TCP mode (`--connect`) and file or directory paths are mutually exclusive.
- Files are read up to 500 MB each (also caps decompression, as a zip-bomb guard).
- Parsed messages remain in memory for filtering and navigation; binary-heavy or compressed logs can require several times their on-disk size in RAM.
- `.zip` archives: only the first entry is read. Zip one `.dlt` per archive, or use `.gz`.
- `E` exports the filtered view to `dlt_export_<timestamp>.txt` in the current working directory.
- Payload text is sanitized before rendering, so logs containing escape sequences can't mess with your terminal.

## Planned

Tracked as [issues](https://github.com/tkmsikd/dlt-tui/issues): bookmarking and annotation, DLT lifecycle/session tracking, and pluggable payload decoders (SOME/IP, UDS).

## Contributing

Issues and PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). `cargo test` runs the whole suite; you can generate `sample.dlt` for manual testing with `cargo test --test generate_sample_dlt -- --ignored --nocapture`.

## License

[MIT](LICENSE)
