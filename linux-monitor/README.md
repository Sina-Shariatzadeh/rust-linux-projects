# Linux Monitor

A Rust command-line tool that shows system information by reading Linux `/proc` files directly.

It shows:

- CPU usage
- RAM and swap usage
- Linux kernel version
- System uptime
- Running processes, sorted by memory use

You can print a one-time report, keep the report refreshing, or open an interactive dashboard built with [ratatui](https://ratatui.rs).

## Example output

```
$ linux-monitor
====== RUST LINUX MONITOR ======

Kernel:    Linux 6.8.0-45-generic
CPU Usage: 24.3% (8 cores)
RAM Usage: 4.2 GB / 16.0 GB (26%)
Swap:      512 MB / 2.0 GB
Uptime:    12 hours, 3 minutes

Running Processes: 143

================================
```

With `--processes 5`, the report also lists the top processes:

```
PID      NAME                 STATE             THREADS        RAM
1024     firefox              S (sleeping)           87     850 MB
2045     code                 S (sleeping)           32     620 MB
...
```

## Build and run

Needs Linux (or WSL) and Rust.

```bash
cargo build --release
./target/release/linux-monitor --help
```

Or run it with Cargo directly:

```bash
cargo run --release -- --tui
```

## Usage

| Command | What it does |
|---|---|
| `linux-monitor` | Prints the report once |
| `linux-monitor --watch` | Refreshes the report every second (Ctrl+C to quit) |
| `linux-monitor --tui` | Opens the dashboard (press `q` or `Esc` to quit) |
| `linux-monitor -p` | Adds the top 20 processes by memory |
| `linux-monitor -p 10` | Adds the top 10 processes |
| `linux-monitor -w -i 500` | Refreshes every 500 ms |

Options:

```
-t, --tui                      Open the interactive dashboard
-w, --watch                    Keep refreshing the plain-text report
-i, --interval <MS>            Refresh interval in milliseconds [default: 1000, minimum: 100]
-p, --processes [<N>]          List the top N processes by memory [default N: 20]
-h, --help                     Print help
-V, --version                  Print version
```

## Where the data comes from

| Information | File | How it is used |
|---|---|---|
| CPU usage | `/proc/stat` | The `cpu` line holds total CPU time counters. Usage is the share of non-idle time between two readings. |
| CPU cores | `/proc/stat` | One `cpuN` line per core |
| RAM and swap | `/proc/meminfo` | Used RAM = `MemTotal` − `MemAvailable`. Used swap = `SwapTotal` − `SwapFree`. |
| Uptime | `/proc/uptime` | First number, in seconds |
| Kernel version | `/proc/version` | The release after `Linux version` |
| Processes | `/proc/[pid]/status` | `Name`, `State`, `Threads` and `VmRSS` (memory in use) |

CPU usage needs two readings, so a one-time report waits about half a second before printing.

## Project structure

```
linux-monitor/
├── Cargo.toml
└── src/
    ├── main.rs      # Command-line options (clap) and the main loop
    ├── linux.rs     # Reads and parses /proc files
    ├── monitor.rs   # Combines the readings into one snapshot and tracks CPU between samples
    └── display.rs   # Text report, size/uptime formatting and the ratatui dashboard
```

Each `read_*` function in `linux.rs` reads a file, and each `parse_*` function works on the text alone. That keeps the parsing easy to test.

## Tests

```bash
cargo test
```

The tests feed in sample `/proc` text, so they run on any OS, including Windows and macOS.

## Libraries

- [clap](https://crates.io/crates/clap): command-line options
- [ratatui](https://crates.io/crates/ratatui): terminal dashboard

## Ideas for next steps

- Per-core CPU bars in the dashboard
- Network traffic from `/proc/net/dev`
- Disk activity from `/proc/diskstats`
- Sorting the process table by CPU
