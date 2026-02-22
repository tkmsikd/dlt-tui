# DLT-TUI Viewer

A fast, keyboard-centric terminal user interface (TUI) for viewing Automotive DLT (Diagnostic Log and Trace) files. Built with Rust and Ratatui.

## Features (v0.1.0 MVP)

- **File Explorer**: Browse and navigate through files and directories natively within the application.
- **Transparent Compression Support**: Automatically reads and decompresses `.dlt`, `.gz`, and `.zip` files containing DLT logs.
- **TUI Viewer**: View parsed DLT messages in an organized table format based on `ratatui`.
- **Syntax Highlighting**: Easily identify warnings and errors. Log rows are colored by Log Level (Fatal=Red, Error=Light Red, Warn=Yellow, Info=Green, Debug=Blue, Verbose=Gray).
- **Vim-like Navigation**: Fast jumps to boundaries and typical `j/k` scrolling.
- **Real-time Filtering**: Drill down logs instantly with multiple input modes:
  - Text filtering (Regex supported) (`/` key)
  - Minimum Log Level filtering (`l` key)
  - Application ID (`APP`) filtering (`a` key)
  - Context ID (`CTX`) filtering (`c` key)

## Installation

Ensure you have [Rust and Cargo installed](https://rustup.rs/). Then run:

```bash
cargo build --release
```

The executable will be located in `target/release/dlt-tui`.

## Usage

You can launch the application with or without setting an initial path:

```bash
# Launch in current directory
cargo run

# Launch in a specific directory
cargo run -- /path/to/logs

# Launch and directly open a specific DLT file (including .gz/.zip)
cargo run -- /path/to/my_log.dlt.gz
```

## Keybindings

### Global / File Explorer

| Key          | Action                          |
| ------------ | ------------------------------- |
| `q`          | Quit the application            |
| `j` / `Down` | Move selection down             |
| `k` / `Up`   | Move selection up               |
| `g` / `Home` | Move to top of list             |
| `G` / `End`  | Move to bottom of list          |
| `Enter`      | Open directory or load log file |

### Log Viewer

| Key   | Action                                                        |
| ----- | ------------------------------------------------------------- |
| `Esc` | Return back to File Explorer                                  |
| `/`   | Open regex string search mode                                 |
| `l`   | Open Minimum Log Level filter mode (Values: F, E, W, I, D, V) |
| `a`   | Open APP ID filter mode                                       |
| `c`   | Open CTX ID filter mode                                       |

_(While in a filter input mode, type your filter query and press `Enter` to apply it, or `Esc` to cancel the filter mode and reset it. Pressing any key will dismiss any error popups.)_

## Future Roadmap

The v0.1.0 release is the MVP marking the initial functional structure of `dlt-tui`.
Further work will implement more sophisticated optimizations like async loading for massive files, advanced custom filtering, packet hex dumps, configuration saves, and more. See `docs/REQUIREMENTS.md` for more info.
