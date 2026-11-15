# Rust Linux Projects

Two command-line tools written in Rust that read system information straight from the Linux `/proc` filesystem. Neither uses a system-info library: each one parses the kernel's files itself.

| Project | What it does | Level |
|---|---|---|
| [linux-monitor](linux-monitor/) | Shows CPU, RAM and swap usage, the kernel version, uptime and running processes. Prints a text report or opens a live terminal dashboard. | Beginner |
| [process-watcher](process-watcher/) | Watches processes in real time. Reports new and exited processes, warns about high CPU or RAM use, and logs everything to a file. | Intermediate |

## Requirements

- Linux, or WSL on Windows. Both tools need `/proc`, so they exit with an error on other systems.
- Rust 1.80 or newer ([install with rustup](https://rustup.rs)).

## Quick start

```bash
git clone <this-repo-url>
cd rust-linux-projects

# System monitor with the live dashboard
cd linux-monitor
cargo run --release -- --tui

# Process watcher: warn above 50% CPU or 500 MB of RAM
cd ../process-watcher
cargo run --release -- -c 50 -m 500
```

## Tests

The parsing code is kept apart from the file reading, so the tests use sample `/proc` text and run on any OS:

```bash
cd linux-monitor && cargo test
cd ../process-watcher && cargo test
```

See each project's README for its options, example output and how it works.
