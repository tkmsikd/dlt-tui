# Contributing to dlt-tui

Thank you for your interest in contributing to dlt-tui! This project aims to provide the automotive software community with a fast, reliable terminal-based DLT log viewer.

## How to Contribute

### Reporting Bugs

If you find a bug, please [open an issue](https://github.com/tkmsikd/dlt-tui/issues/new) with:

- A clear description of the problem
- Steps to reproduce (if possible, include a sample `.dlt` file)
- Your environment (OS, terminal emulator, Rust version)

### Suggesting Features

Feature requests are welcome! Please check the [Roadmap](README.md#roadmap) first to see if your idea is already planned. If not, [open an issue](https://github.com/tkmsikd/dlt-tui/issues/new) describing:

- The use case (e.g., "When debugging CAN bus issues, I need to...")
- The expected behavior
- Any reference to how other tools handle it

### Pull Requests

1. **Fork** the repository and create a new branch from `master`
2. **Write tests first** — this project follows TDD principles
3. **Keep commits atomic** — one logical change per commit, using [Conventional Commits](https://www.conventionalcommits.org/)
4. **Run all checks** before submitting:

```bash
# Run tests
cargo test

# Run clippy
cargo clippy --all-targets

# Check formatting
cargo fmt --check
```

5. **Open a PR** with a clear description of what you changed and why

## Development Setup

```bash
git clone https://github.com/tkmsikd/dlt-tui.git
cd dlt-tui
cargo build
cargo test
```

### Generating sample DLT files for testing

```bash
cargo test --test generate_sample_dlt -- --nocapture
# Creates sample.dlt in the project root
```

## Code Style

- Follow standard Rust conventions and `rustfmt` defaults
- Use `clippy` to catch common issues
- Prefer `Result`/`Option` over `unwrap()` in production code
- Sanitize all external input (DLT payloads may contain malicious data)

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
