# Process Watcher

A real-time Linux process monitor written in Rust. It reads `/proc/[PID]` files directly to:

- List running processes with their PID, name, CPU and RAM use
- Detect new processes as they start, and processes that exit
- Warn when a process uses too much CPU or RAM
- Save all activity to a log file with timestamps

It runs on [tokio](https://tokio.rs) and refreshes on a timer.

## Example output

```
====== PROCESS WATCHER ======   14:03:22   (312 processes, every 2000 ms)

PID     NAME              S       CPU        RAM
1024    firefox           S     92.1%     850 MB
2045    code              S      8.0%     620 MB
3012    bash              S      1.0%      12 MB

[14:03:18] [!] High CPU usage: PID 1024 (firefox) 92.1%
[14:03:20] [+] New process detected: nginx (PID 4242)
[14:03:21] [-] Process exited: sleep (PID 4100)

=============================
Logging to process_watcher.log · Ctrl+C to quit
```

The log file has more detail, including the executable path and full command line:

```
2026-11-15 14:03:18 [!] CPU   pid=1024 name=firefox cpu=92.1%
2026-11-15 14:03:20 [+] START pid=4242 ppid=1 name=nginx exe=/usr/sbin/nginx cmd=nginx -g daemon off;
2026-11-15 14:03:21 [-] EXIT  pid=4100 name=sleep
```

## Build and run

Needs Linux (or WSL) and Rust.

```bash
cargo build --release
./target/release/process-watcher --help
```

Or run it with Cargo directly:

```bash
cargo run --release -- -c 80 -m 500
```

Run it with `sudo` to see the executable path (`/proc/[PID]/exe`) of processes owned by other users. Without it, those show as `?` in the log.

## Usage

| Command | What it does |
|---|---|
| `process-watcher` | Watches every process and refreshes every 2 seconds |
| `process-watcher -c 80` | Warns when a process goes above 80% CPU |
| `process-watcher -m 500` | Warns when a process uses more than 500 MB of RAM |
| `process-watcher -f nginx` | Only watches processes with "nginx" in their name or command line |
| `process-watcher -q` | Prints events only, without the table (good for piping to other tools) |
| `process-watcher -l /tmp/activity.log` | Writes the log somewhere else |

Options:

```
-i, --interval <MS>            Refresh interval in milliseconds [default: 2000, minimum: 100]
-c, --cpu-threshold <PERCENT>  CPU % that triggers a warning (100 = one full core) [default: 50]
-m, --mem-threshold <MB>       RAM in MB that triggers a warning [default: off]
-n, --top <N>                  Number of processes shown in the table [default: 15]
-l, --log <FILE>               Log file [default: process_watcher.log]
-f, --filter <TEXT>            Only watch processes whose name or command line contains TEXT
-q, --quiet                    Print events only, without the table
-h, --help                     Print help
-V, --version                  Print version
```

## How it works

### Where the data comes from

| File | Used for |
|---|---|
| `/proc/[PID]/stat` | Name, state, parent PID, CPU time (`utime` + `stime`) and start time |
| `/proc/[PID]/status` | Memory in use (`VmRSS`) |
| `/proc/[PID]/cmdline` | Full command line. Arguments are separated by NUL bytes. |
| `/proc/[PID]/exe` | Link to the executable file |
| `/proc/stat` | Total CPU time and number of cores, used to work out each process's CPU % |

### CPU usage

Each refresh compares a process's CPU time with the previous reading:

```
CPU % = (process CPU time used) / (total CPU time passed) × cores × 100
```

This matches `top`: 100% means one core is fully busy, so a multi-threaded program can go above 100%.

### Detecting new and exited processes

The watcher remembers every process from the previous refresh:

- A PID that wasn't there before is a **new process** `[+]`.
- A PID that has disappeared is an **exited process** `[-]`.
- Linux reuses PIDs. If a PID's start time has changed, the old process exited and a new one took its number. The watcher reports both.

The first refresh only records what is already running, so you don't get hundreds of "new process" lines at startup. The RAM warning is the exception: programs already using too much memory are flagged right away.

### Avoiding repeated warnings

A warning appears once when a process crosses the limit. It can only appear again after usage drops below 80% of the limit. This stops a process that hovers around the limit from filling the screen.

### Async design

- A `tokio` timer starts each refresh.
- Reading hundreds of `/proc` files is blocking work, so it runs in `spawn_blocking` and doesn't stall the async runtime.
- The log is written with `tokio::fs`.
- Ctrl+C is caught with `tokio::signal`, so the watcher writes `watcher stopped` to the log before exiting.

## Project structure

```
process-watcher/
├── Cargo.toml
└── src/
    ├── main.rs      # Command-line options, tokio main loop, Ctrl+C handling
    ├── linux.rs     # Reads and parses /proc/[PID] files and /proc/stat
    ├── monitor.rs   # Tracks processes between refreshes: CPU %, start/exit detection, warnings
    ├── display.rs   # Draws the table and recent events
    └── logger.rs    # Writes timestamped lines to the log file (tokio + chrono)
```

## Tests

```bash
cargo test
```

The tests run on any OS. They cover:

- Parsing `stat` lines, including names with spaces or `)`
- Parsing `status`, `cmdline` and `/proc/stat`
- Detecting new and exited processes, including reused PIDs
- CPU % calculation and one-time warnings
- The name filter and the screen output

## Libraries

- [tokio](https://crates.io/crates/tokio): async timer, file writing and Ctrl+C handling
- [chrono](https://crates.io/crates/chrono): timestamps
- [clap](https://crates.io/crates/clap): command-line options

## Ideas for next steps

- Show a process tree using the parent PID
- Track open files from `/proc/[PID]/fd`
- Send warnings to a webhook or desktop notification
- Save logs as JSON for other tools to read
